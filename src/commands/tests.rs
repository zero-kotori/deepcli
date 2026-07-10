use super::*;
use crate::agents::{AgentStore, SubagentStatus};
use crate::config::AppConfig;
use crate::config::ProviderCredentials;
use crate::config::{GitIdentityConfig, ProviderConfig};
use crate::permissions::PermissionEngine;
use crate::prompts::PromptStore;
use crate::session::ApprovalRequest;
use crate::session::AuditEvent;
use crate::session::SideQuestion;
use crate::session::{Plan, PlanStep};
use crate::skills::SkillStore;
use crate::tools::{CommandOutput, EnvironmentCheck};
use approval::format_approval_requests;
use btw::format_side_questions;
use completion::{
    completion_install_target, format_completion_install_json, format_completion_status_json,
    install_completion_script_in,
};
use credentials::handle_credentials;
use delivery_diff::{
    filter_diff_by_paths, format_diff_name_only, format_diff_stat, parse_diff_args,
    parse_review_args, DiffView,
};
use delivery_reports::{
    format_handoff_report, format_handoff_report_json, format_handoff_report_pr_description,
    format_verification_report, format_verification_report_json, review_risk_summary_from_report,
    weak_test_command_reason, HandoffReportInput, VerificationDiffSource,
    VerificationEnvironmentCheck, VerificationReportInput, VerificationStatusSource,
    VerificationTestRun,
};
use delivery_review::{review_diff, review_worktree};
use delivery_verify::{parse_handoff_args, parse_verify_args, HandoffFormat};
use diagnose::parse_diagnose_options;
use doctor::{
    apply_doctor_fixes, doctor_next_actions, doctor_shell_next_actions,
    expected_deepcli_workspace_paths, format_shell_command_status, parse_doctor_options,
    probe_provider, provider_readiness_reports, record_provider_probe, shell_command_status_in,
    DoctorOptions, ProviderProbeReport,
};
use env::{
    format_environment_check_json, format_environment_plan, format_environment_plan_json,
    format_environment_setup_result_json, format_environment_test_run_json, parse_env_options,
};
use model::model_list_text;
use preflight::{
    format_preflight_json, format_preflight_text, preflight_next_actions, PreflightCheckResult,
    PreflightOptions, PreflightReport,
};
use privacy::{redacted_user_home, USER_HOME_PREFIX};
use round_report::{format_round_text, RoundTextInput};
use scorecard_report::{
    build_scorecard_report, scorecard_summary_json, SCORECARD_BENCHMARK_REMEDIATION_ACTION,
};
use serde_json::{json, Value};
use session_export::parse_export_args;
use session_selection::parse_limit_and_session_selection;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use tempfile::tempdir;
use terminal::{terminal_next_actions, terminal_workspace_command, DEFAULT_TERMINAL_APP};
use trace::format_audit_trace;
use usage::{format_usage_diagnostics, summarize_audit_usage};
use web::web_search_query_from_args;

static TERMINAL_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn write_round_scorecard_ready_fixture(workspace: &Path) {
    fs::create_dir_all(workspace.join("docs/ai")).unwrap();
    fs::create_dir_all(workspace.join("docs")).unwrap();
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::create_dir_all(workspace.join(".deepcli")).unwrap();
    fs::write(
        workspace.join("docs/ai/REQUIREMENTS.md"),
        "# Requirements\n",
    )
    .unwrap();
    fs::write(workspace.join("docs/ai/TECHNICAL_PLAN.md"), "# Plan\n").unwrap();
    fs::write(workspace.join("docs/FEATURES.md"), "# Features\n").unwrap();
    fs::write(workspace.join("src/ui.rs"), "// test fixture\n").unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(workspace.join(".deepcli/config.json"), "{}\n").unwrap();
}

fn write_round_ready_benchmark_history(workspace: &Path) {
    let now = Utc::now();
    for sample in 0..2 {
        for (index, preset) in MEANINGFUL_BENCHMARK_PRESETS.iter().enumerate() {
            write_benchmark_status_test_artifact(
                workspace,
                &format!(
                    "2099010{}T00000{}Z-product-{preset}.json",
                    sample + 1,
                    index
                ),
                now + chrono::Duration::seconds((sample * 10 + index) as i64),
                preset,
                preset,
                "passed",
            );
        }
    }
}

fn write_ready_competitor_baseline(workspace: &Path) {
    let baseline = workspace.join(".deepcli/baselines/competitor.json");
    fs::create_dir_all(baseline.parent().unwrap()).unwrap();
    fs::write(
        baseline,
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "competitor",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": "passed",
                    "durationMs": 140
                },
                {
                    "suite": "product",
                    "case": "preflight-quick",
                    "status": "passed",
                    "durationMs": 280
                },
                {
                    "suite": "product",
                    "case": "selftest",
                    "status": "passed",
                    "durationMs": 35
                },
                {
                    "suite": "product",
                    "case": "scorecard",
                    "status": "passed",
                    "durationMs": 12
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn parses_core_slash_commands() {
    assert_eq!(CommandRouter::parse("hello").unwrap(), None);
    assert_eq!(
        CommandRouter::parse("/help").unwrap(),
        Some(SlashCommand::Help { args: Vec::new() })
    );
    assert_eq!(
        CommandRouter::parse("/help env").unwrap(),
        Some(SlashCommand::Help {
            args: vec!["env".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/version --json").unwrap(),
        Some(SlashCommand::Version {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/quickstart").unwrap(),
        Some(SlashCommand::Quickstart { args: Vec::new() })
    );
    assert_eq!(
        CommandRouter::parse("/quickstart --json").unwrap(),
        Some(SlashCommand::Quickstart {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/quickstart --fail-on-missing").unwrap(),
        Some(SlashCommand::Quickstart {
            args: vec!["--fail-on-missing".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/recipes release --json").unwrap(),
        Some(SlashCommand::Recipes {
            args: vec!["release".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/scorecard --json").unwrap(),
        Some(SlashCommand::Scorecard {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/opportunities --json").unwrap(),
        Some(SlashCommand::Opportunities {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/round --json").unwrap(),
        Some(SlashCommand::Round {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark --fail-below 85").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec!["--fail-below".to_string(), "85".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark record --json --scorecard").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec![
                "record".to_string(),
                "--json".to_string(),
                "--scorecard".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark run --json --command 'printf ok'").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec![
                "run".to_string(),
                "--json".to_string(),
                "--command".to_string(),
                "printf ok".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark run --preset cargo-test --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec![
                "run".to_string(),
                "--preset".to_string(),
                "cargo-test".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark run-suite --preset smoke --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec![
                "run-suite".to_string(),
                "--preset".to_string(),
                "smoke".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark presets --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec!["presets".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark status --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec!["status".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark gate --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec!["gate".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark summary --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec!["summary".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark trends --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec!["trends".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/benchmark clean --dry-run --json").unwrap(),
        Some(SlashCommand::Benchmark {
            args: vec![
                "clean".to_string(),
                "--dry-run".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/selftest --json --fail-on-issues").unwrap(),
        Some(SlashCommand::Selftest {
            args: vec!["--json".to_string(), "--fail-on-issues".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/preflight --json").unwrap(),
        Some(SlashCommand::Preflight {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/completion zsh --output .deepcli/exports/_deepcli").unwrap(),
        Some(SlashCommand::Completion {
            args: vec![
                "zsh".to_string(),
                "--output".to_string(),
                ".deepcli/exports/_deepcli".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/completion install zsh --force --json").unwrap(),
        Some(SlashCommand::Completion {
            args: vec![
                "install".to_string(),
                "zsh".to_string(),
                "--force".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/completion status zsh --json").unwrap(),
        Some(SlashCommand::Completion {
            args: vec![
                "status".to_string(),
                "zsh".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/init --probe-provider").unwrap(),
        Some(SlashCommand::Init {
            args: vec!["--probe-provider".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/usage --json --output .deepcli/exports/usage.json abc").unwrap(),
        Some(SlashCommand::Usage {
            args: vec![
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/usage.json".to_string(),
                "abc".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/status --json").unwrap(),
        Some(SlashCommand::Status {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/usage abc").unwrap(),
        Some(SlashCommand::Usage {
            args: vec!["abc".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/compiler setup --smoke").unwrap(),
        Some(SlashCommand::Env {
            args: vec![
                "setup".to_string(),
                "compiler".to_string(),
                "--smoke".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/doctor").unwrap(),
        Some(SlashCommand::Doctor { args: Vec::new() })
    );
    assert_eq!(
        CommandRouter::parse("/doctor --fix").unwrap(),
        Some(SlashCommand::Doctor {
            args: vec!["--fix".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/doctor --quick").unwrap(),
        Some(SlashCommand::Doctor {
            args: vec!["--quick".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/doctor docker --json").unwrap(),
        Some(SlashCommand::Env {
            args: vec![
                "check".to_string(),
                "docker".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse(
            "/doctor --probe-provider --provider kimi --json --output .deepcli/exports/doctor.json"
        )
        .unwrap(),
        Some(SlashCommand::Doctor {
            args: vec![
                "--probe-provider".to_string(),
                "--provider".to_string(),
                "kimi".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/doctor.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/diagnose --limit 3 --full-env").unwrap(),
        Some(SlashCommand::Diagnose {
            args: vec![
                "--limit".to_string(),
                "3".to_string(),
                "--full-env".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/diagnose compiler --json").unwrap(),
        Some(SlashCommand::Env {
            args: vec![
                "check".to_string(),
                "compiler".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/support").unwrap(),
        Some(SlashCommand::Diagnose {
            args: vec![
                "--bundle".to_string(),
                DEFAULT_SUPPORT_BUNDLE_DIR.to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/support .deepcli/support/slow-run --json").unwrap(),
        Some(SlashCommand::Diagnose {
            args: vec![
                "--bundle".to_string(),
                ".deepcli/support/slow-run".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/support --full-env").unwrap(),
        Some(SlashCommand::Diagnose {
            args: vec![
                "--bundle".to_string(),
                DEFAULT_SUPPORT_BUNDLE_DIR.to_string(),
                "--full-env".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/verify --env-check compiler --json").unwrap(),
        Some(SlashCommand::Verify {
            args: vec![
                "--env-check".to_string(),
                "compiler".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/config get agent.providerTurnTimeoutSeconds").unwrap(),
        Some(SlashCommand::Config {
            args: vec![
                "get".to_string(),
                "agent.providerTurnTimeoutSeconds".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/timeout 900").unwrap(),
        Some(SlashCommand::Timeout {
            args: vec!["900".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/timeout --json").unwrap(),
        Some(SlashCommand::Timeout {
            args: vec!["--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/trace --limit 5").unwrap(),
        Some(SlashCommand::Trace {
            args: vec!["--limit".to_string(), "5".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/trace --json --output .deepcli/exports/trace.json").unwrap(),
        Some(SlashCommand::Trace {
            args: vec![
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/trace.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/logs --limit 5 --json").unwrap(),
        Some(SlashCommand::Logs {
            args: vec!["--limit".to_string(), "5".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/privacy --json --output .deepcli/exports/privacy.json").unwrap(),
        Some(SlashCommand::Privacy {
            args: vec![
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/privacy.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/diff --staged").unwrap(),
        Some(SlashCommand::Diff {
            args: vec!["--staged".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/review --path src").unwrap(),
        Some(SlashCommand::Review {
            args: vec!["--path".to_string(), "src".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/accept --json").unwrap(),
        Some(SlashCommand::Verify {
            args: vec!["--json".to_string(), "--run-tests".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/accept --test-command 'cargo test'").unwrap(),
        Some(SlashCommand::Verify {
            args: vec!["--test-command".to_string(), "cargo test".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/gate --json -- cargo test").unwrap(),
        Some(SlashCommand::Verify {
            args: vec![
                "--json".to_string(),
                "--fail-on-blockers".to_string(),
                "--".to_string(),
                "cargo".to_string(),
                "test".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/verify --limit 3").unwrap(),
        Some(SlashCommand::Verify {
            args: vec!["--limit".to_string(), "3".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/handoff --path src").unwrap(),
        Some(SlashCommand::Handoff {
            args: vec!["--path".to_string(), "src".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/handoff --env-check docker --json").unwrap(),
        Some(SlashCommand::Handoff {
            args: vec![
                "--env-check".to_string(),
                "docker".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/resume abc").unwrap(),
        Some(SlashCommand::Resume {
            args: vec!["abc".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/resume abc --dry-run --json").unwrap(),
        Some(SlashCommand::Resume {
            args: vec![
                "abc".to_string(),
                "--dry-run".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/rename compiler fix").unwrap(),
        Some(SlashCommand::Rename {
            args: vec!["compiler".to_string(), "fix".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/stop").unwrap(),
        Some(SlashCommand::Stop)
    );
    assert_eq!(
        CommandRouter::parse("/quit").unwrap(),
        Some(SlashCommand::Quit)
    );
    assert_eq!(
        CommandRouter::parse("/permissions set-mode write").unwrap(),
        Some(SlashCommand::Permissions {
            args: vec!["set-mode".to_string(), "write".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/permissions show --json --output .deepcli/exports/permissions.json")
            .unwrap(),
        Some(SlashCommand::Permissions {
            args: vec![
                "show".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/permissions.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/credentials import-env deepseek --force").unwrap(),
        Some(SlashCommand::Credentials {
            args: vec![
                "import-env".to_string(),
                "deepseek".to_string(),
                "--force".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/credentials set deepseek --stdin --force").unwrap(),
        Some(SlashCommand::Credentials {
            args: vec![
                "set".to_string(),
                "deepseek".to_string(),
                "--stdin".to_string(),
                "--force".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/logout deepseek").unwrap(),
        Some(SlashCommand::Credentials {
            args: vec!["remove".to_string(), "deepseek".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/test run cargo test").unwrap(),
        Some(SlashCommand::Test {
            args: vec!["run".to_string(), "cargo".to_string(), "test".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/test discover --json --output .deepcli/exports/tests.json").unwrap(),
        Some(SlashCommand::Test {
            args: vec![
                "discover".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/tests.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse(
            "/test run --json --output .deepcli/exports/test-run.json -- cargo test"
        )
        .unwrap(),
        Some(SlashCommand::Test {
            args: vec![
                "run".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/test-run.json".to_string(),
                "--".to_string(),
                "cargo".to_string(),
                "test".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/install compiler --smoke --json").unwrap(),
        Some(SlashCommand::Env {
            args: vec![
                "install".to_string(),
                "compiler".to_string(),
                "--smoke".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/web search rust ownership").unwrap(),
        Some(SlashCommand::Web {
            args: vec![
                "search".to_string(),
                "rust".to_string(),
                "ownership".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/context").unwrap(),
        Some(SlashCommand::Context)
    );
    assert_eq!(
        CommandRouter::parse("/goal 完整实现 docs 需求 --json").unwrap(),
        Some(SlashCommand::Goal {
            args: vec![
                "完整实现".to_string(),
                "docs".to_string(),
                "需求".to_string(),
                "--json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/goal status --json").unwrap(),
        Some(SlashCommand::Goal {
            args: vec!["status".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/goal gate --json").unwrap(),
        Some(SlashCommand::Goal {
            args: vec!["gate".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/plan 做一个需求澄清工具").unwrap(),
        Some(SlashCommand::Plan {
            args: vec!["做一个需求澄清工具".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/fork --current --dry-run --verify").unwrap(),
        Some(SlashCommand::Fork {
            args: vec![
                "--current".to_string(),
                "--dry-run".to_string(),
                "--verify".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/terminal --dry-run --json").unwrap(),
        Some(SlashCommand::Terminal {
            args: vec!["--dry-run".to_string(), "--json".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/cmd echo \"$HOME\" | sed 's#/#:#g'").unwrap(),
        Some(SlashCommand::Cmd {
            command: "echo \"$HOME\" | sed 's#/#:#g'".to_string(),
            attach: false
        })
    );
    assert_eq!(
        CommandRouter::parse("/cmd --attach git status --short").unwrap(),
        Some(SlashCommand::Cmd {
            command: "git status --short".to_string(),
            attach: true
        })
    );
    assert_eq!(
        CommandRouter::parse("/model set deepseek deepseek-v4-pro").unwrap(),
        Some(SlashCommand::Model {
            args: vec![
                "set".to_string(),
                "deepseek".to_string(),
                "deepseek-v4-pro".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/model kimi").unwrap(),
        Some(SlashCommand::Model {
            args: vec!["set".to_string(), "kimi".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/model list --json --output .deepcli/exports/models.json").unwrap(),
        Some(SlashCommand::Model {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/models.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/prompt list --json --output .deepcli/exports/prompts.json").unwrap(),
        Some(SlashCommand::Prompt {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/prompts.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/skill list --json --output .deepcli/exports/skills.json").unwrap(),
        Some(SlashCommand::Skill {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/skills.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/session show abc").unwrap(),
        Some(SlashCommand::Session {
            args: vec!["show".to_string(), "abc".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/session list --json --output .deepcli/exports/sessions.json")
            .unwrap(),
        Some(SlashCommand::Session {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/sessions.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/cleanup").unwrap(),
        Some(SlashCommand::Session {
            args: vec!["prune-empty".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/cleanup sessions --json --output .deepcli/exports/cleanup.json")
            .unwrap(),
        Some(SlashCommand::Session {
            args: vec![
                "prune-empty".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/cleanup.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse(
            "/session search compiler --json --output .deepcli/exports/session-search.json"
        )
        .unwrap(),
        Some(SlashCommand::Session {
            args: vec![
                "search".to_string(),
                "compiler".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/session-search.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/session history --limit 5 abc").unwrap(),
        Some(SlashCommand::Session {
            args: vec![
                "history".to_string(),
                "--limit".to_string(),
                "5".to_string(),
                "abc".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse(
            "/session history --limit 5 --json --output .deepcli/exports/session-history.json abc"
        )
        .unwrap(),
        Some(SlashCommand::Session {
            args: vec![
                "history".to_string(),
                "--limit".to_string(),
                "5".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/session-history.json".to_string(),
                "abc".to_string()
            ]
        })
    );
    assert_eq!(
            CommandRouter::parse(
                "/session diagnose --limit 3 --json --output .deepcli/exports/session-diagnose.json abc"
            )
            .unwrap(),
            Some(SlashCommand::Session {
                args: vec![
                    "diagnose".to_string(),
                    "--limit".to_string(),
                    "3".to_string(),
                    "--json".to_string(),
                    "--output".to_string(),
                    ".deepcli/exports/session-diagnose.json".to_string(),
                    "abc".to_string()
                ]
            })
        );
    assert_eq!(
        CommandRouter::parse("/agent list").unwrap(),
        Some(SlashCommand::Agent {
            args: vec!["list".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/agent list --json --output .deepcli/exports/agents.json").unwrap(),
        Some(SlashCommand::Agent {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/agents.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/btw ask quick question").unwrap(),
        Some(SlashCommand::Btw {
            args: vec![
                "ask".to_string(),
                "quick".to_string(),
                "question".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/btw list --json --output .deepcli/exports/btw.json").unwrap(),
        Some(SlashCommand::Btw {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/btw.json".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/approval approve abc").unwrap(),
        Some(SlashCommand::Approval {
            args: vec!["approve".to_string(), "abc".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/approval list --json --output .deepcli/exports/approvals.json")
            .unwrap(),
        Some(SlashCommand::Approval {
            args: vec![
                "list".to_string(),
                "--json".to_string(),
                "--output".to_string(),
                ".deepcli/exports/approvals.json".to_string()
            ]
        })
    );
}

#[test]
fn slash_cmd_requires_a_shell_command() {
    let error = CommandRouter::parse("/cmd").unwrap_err().to_string();
    assert!(error.contains("missing shell command for /cmd"));
}

#[test]
fn goal_command_creates_contract_and_guard_plan() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "实现".to_string(),
            "全部文档需求".to_string(),
            "--acceptance-cmd".to_string(),
            "cargo test --all".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.goal.v1");
    assert_eq!(value["status"], "created");
    assert!(value["goal"]["objective"]
        .as_str()
        .unwrap()
        .contains("全部文档需求"));
    assert!(value["goal"]["acceptance_commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str() == Some("cargo test --all")));

    let loaded = store.load(&session.id().to_string()).unwrap();
    assert!(loaded.load_goal().unwrap().is_some());
    let plan = loaded.load_plan().unwrap().unwrap();
    assert!(plan.steps.iter().any(|step| step.id == "goal_tests"));
}

#[test]
fn goal_command_supports_codex_style_lifecycle_controls_and_budget() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "start".to_string(),
            "完成迁移并保持测试通过".to_string(),
            "--token-budget".to_string(),
            "1200".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["goal"]["status"], "active");
    assert_eq!(value["goal"]["token_budget"], 1200);
    assert_eq!(value["goal"]["tokens_used"], 0);
    assert_eq!(value["goal"]["time_used_seconds"], 0);

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["pause".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["status"], "paused");
    assert_eq!(value["goal"]["status"], "paused");

    let error = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["gate".to_string(), "--json".to_string()],
    )
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    assert!(value["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("Paused")));

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["resume".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["status"], "resumed");
    assert_eq!(value["goal"]["status"], "active");

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "edit".to_string(),
            "完成迁移、更新文档并保持测试通过".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["status"], "updated");
    assert!(value["goal"]["objective"]
        .as_str()
        .unwrap()
        .contains("更新文档"));

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["block".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["goal"]["status"], "blocked");
}

#[test]
fn goal_gate_fails_until_plan_and_acceptance_evidence_are_complete() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "实现全部需求".to_string(),
            "--acceptance-cmd".to_string(),
            "cargo test".to_string(),
        ],
    )
    .unwrap();

    let error = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["gate".to_string(), "--json".to_string()],
    )
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(value["schema"], "deepcli.goal.status.v1");
    assert_eq!(value["ready"], false);
    assert!(value["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("plan has")));
    assert!(value["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("cargo test")));
}

#[test]
fn goal_complete_requires_readiness_and_marks_ready_goal_complete() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "# test\n").unwrap();
    fs::create_dir_all(dir.path().join("docs/ai")).unwrap();
    fs::write(dir.path().join("docs/FEATURES.md"), "# test\n").unwrap();
    fs::write(dir.path().join("docs/ai/REQUIREMENTS.md"), "# test\n").unwrap();
    fs::write(dir.path().join("docs/ai/TECHNICAL_PLAN.md"), "# test\n").unwrap();
    fs::write(dir.path().join("docs/ai/CONTEXT.md"), "# test\n").unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "实现全部需求".to_string(),
            "--acceptance-cmd".to_string(),
            "cargo test".to_string(),
        ],
    )
    .unwrap();

    let error = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["complete".to_string(), "--json".to_string()],
    )
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["goal"]["status"], "active");

    let loaded = store.load(&session.id().to_string()).unwrap();
    let goal = loaded.load_goal().unwrap().unwrap();
    for step in loaded.load_plan().unwrap().unwrap().steps {
        loaded
            .update_plan_step(&step.id, PlanStepStatus::Completed)
            .unwrap();
    }
    for command in goal.acceptance_commands {
        loaded
            .append_test_run(&TestRunRecord {
                command,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                passed: true,
                created_at: Utc::now(),
            })
            .unwrap();
    }

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["complete".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["status"], "completed");
    assert_eq!(value["goal"]["status"], "complete");

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["gate".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["ready"], true);
    assert_eq!(value["goal"]["status"], "complete");
}

#[test]
fn goal_status_and_gate_fall_back_to_latest_session_with_goal() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let goal_session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    handle_goal(
        dir.path(),
        Some(goal_session.id().to_string()),
        vec![
            "实现全部需求".to_string(),
            "--acceptance-cmd".to_string(),
            "cargo test".to_string(),
        ],
    )
    .unwrap();
    let empty_current = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    let status = handle_goal(
        dir.path(),
        None,
        vec!["status".to_string(), "--json".to_string()],
    )
    .unwrap();
    let status_value: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status_value["schema"], "deepcli.goal.status.v1");
    assert_eq!(status_value["sessionSource"], "latest_with_goal");
    assert_eq!(status_value["session"]["id"], goal_session.id().to_string());

    let error = handle_goal(
        dir.path(),
        Some(empty_current.id().to_string()),
        vec!["gate".to_string(), "--json".to_string()],
    )
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);
    let gate_value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(gate_value["sessionSource"], "latest_with_goal");
    assert_eq!(gate_value["session"]["id"], goal_session.id().to_string());
    assert_eq!(gate_value["ready"], false);
}

#[test]
fn goal_creation_still_requires_active_session() {
    let dir = tempdir().unwrap();
    let error = handle_goal(dir.path(), None, Vec::new()).unwrap_err();
    assert!(error.to_string().contains("requires an active session"));
}

#[test]
fn goal_gate_passes_when_plan_and_acceptance_evidence_are_complete() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "# test\n").unwrap();
    fs::create_dir_all(dir.path().join("docs/ai")).unwrap();
    fs::write(dir.path().join("docs/FEATURES.md"), "# test\n").unwrap();
    fs::write(dir.path().join("docs/ai/REQUIREMENTS.md"), "# test\n").unwrap();
    fs::write(dir.path().join("docs/ai/TECHNICAL_PLAN.md"), "# test\n").unwrap();
    fs::write(dir.path().join("docs/ai/CONTEXT.md"), "# test\n").unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "实现全部需求".to_string(),
            "--acceptance-cmd".to_string(),
            "cargo test".to_string(),
            "--acceptance-cmd".to_string(),
            "./scripts/deepcli preflight --json".to_string(),
        ],
    )
    .unwrap();

    let loaded = store.load(&session.id().to_string()).unwrap();
    let goal = loaded.load_goal().unwrap().unwrap();
    for step in loaded.load_plan().unwrap().unwrap().steps {
        loaded
            .update_plan_step(&step.id, PlanStepStatus::Completed)
            .unwrap();
    }
    for command in goal.acceptance_commands {
        loaded
            .append_test_run(&TestRunRecord {
                command,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                passed: true,
                created_at: Utc::now(),
            })
            .unwrap();
    }

    let output = handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec!["gate".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.goal.status.v1");
    assert_eq!(value["ready"], true);
    assert!(value["blockers"].as_array().unwrap().is_empty());
}

#[test]
fn plan_command_show_reads_saved_plan_and_rejects_local_draft_generation() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_plan(&Plan {
            title: "Model-backed planning".to_string(),
            steps: vec![PlanStep {
                id: "context".to_string(),
                description: "Inspect code context".to_string(),
                status: PlanStepStatus::Pending,
            }],
            updated_at: Utc::now(),
        })
        .unwrap();

    let output = handle_plan_command(
        dir.path(),
        Some(session.id().to_string()),
        vec!["show".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["title"], "Model-backed planning");

    session
        .write_plan_document("# Generated Plan\n\n### Critical Files for Implementation\n- src/runtime.rs\n- src/session.rs\n- src/commands/plan.rs\n")
        .unwrap();
    let output = handle_plan_command(
        dir.path(),
        Some(session.id().to_string()),
        vec!["show".to_string()],
    )
    .unwrap();
    assert!(output.contains("# Generated Plan"));
    assert!(output.contains("src/runtime.rs"));

    let error = handle_plan_command(
        dir.path(),
        Some(session.id().to_string()),
        vec!["支持交互式需求澄清".to_string()],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("model-backed planning flow"));
}

#[test]
fn fork_command_clones_session_context_without_opening_terminal() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.rename("original task").unwrap();
    session.append_message("user", "hello").unwrap();
    session.append_message("assistant", "world").unwrap();
    session.set_state(SessionState::WaitingUser).unwrap();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--no-open".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.fork.v1");
    assert_eq!(value["terminal"]["opened"], false);
    assert_eq!(value["contextCopy"]["mode"], "persisted_session_files");
    assert_eq!(value["contextCopy"]["hotForkSupported"], false);
    assert_eq!(value["contextCopy"]["sourceState"], "waiting_user");
    assert_eq!(value["contextCopy"]["completeForIdleSession"], true);
    let fork_id = value["fork"]["id"].as_str().unwrap();
    assert_ne!(fork_id, session.id().to_string());
    let workspace_resume_command = value["terminal"]["workspaceResumeCommand"]
        .as_str()
        .expect("fork JSON should include workspace-aware resume command");
    assert!(workspace_resume_command.starts_with("cd "));
    assert!(workspace_resume_command.contains(" && deepcli resume "));
    assert!(workspace_resume_command.ends_with(fork_id));
    assert_eq!(value["nextActions"][0], workspace_resume_command);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Resume forked context".to_string()));
    assert!(checklist_labels.contains(&"Resume saved work".to_string()));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("workspace resume command: cd "));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains(&format!("  - {workspace_resume_command}")));

    let fork = store.load(fork_id).unwrap();
    assert_eq!(fork.load_messages().unwrap().len(), 2);
    assert!(fork.metadata.title.as_deref().unwrap().contains("Fork of"));
    assert_eq!(fork.metadata.state, SessionState::WaitingUser);
}

#[test]
fn fork_verify_json_reports_resume_health_for_created_clone() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.rename("debug long context").unwrap();
    session.append_message("user", "hello").unwrap();
    session.append_message("assistant", "world").unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "read_file".to_string(),
            input: json!({"path": "src/main.rs"}),
            output: json!({"path": "src/main.rs"}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    session.set_state(SessionState::WaitingUser).unwrap();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--no-open".to_string(),
            "--verify".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.fork.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["dryRun"], false);
    assert_eq!(value["verification"]["status"], "ok");
    assert_eq!(value["verification"]["resumeReady"], true);
    assert_eq!(value["verification"]["sameWorkspace"], true);
    assert_eq!(value["verification"]["providerMatches"], true);
    assert_eq!(value["verification"]["modelMatches"], true);
    assert_eq!(value["verification"]["messageCount"]["source"], 2);
    assert_eq!(value["verification"]["messageCount"]["fork"], 2);
    assert_eq!(value["verification"]["messageCount"]["matches"], true);
    assert_eq!(value["verification"]["toolCount"]["source"], 1);
    assert_eq!(value["verification"]["toolCount"]["fork"], 1);
    assert_eq!(value["verification"]["toolCount"]["matches"], true);
    assert_eq!(value["verification"]["forkState"], "waiting_user");
    assert!(value["verification"]["resumeCommand"]
        .as_str()
        .unwrap()
        .starts_with("deepcli resume "));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("verification: ok"));
}

#[test]
fn fork_report_warns_when_source_session_is_running() {
    let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
    let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
    let _term_guard = EnvVarGuard::remove("TERM_PROGRAM");
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.append_message("user", "long running task").unwrap();
    session.set_state(SessionState::Executing).unwrap();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--no-open".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["contextCopy"]["sourceState"], "executing");
    assert_eq!(value["contextCopy"]["completeForIdleSession"], false);
    assert_eq!(value["contextCopy"]["runningAgentState"], true);
    assert!(value["contextCopy"]["warning"]
        .as_str()
        .unwrap()
        .contains("does not copy the in-memory running agent"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_shell_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Stop running task".to_string()));
    assert!(checklist_labels.contains(&"Fork active context".to_string()));
    assert!(next_actions.iter().any(|action| action == "deepcli stop"));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli fork --current"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("source state: executing"));

    let preview = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let preview_value: Value = serde_json::from_str(&preview).unwrap();
    let preview_next_actions = json_string_array(&preview_value["nextActions"]);
    assert_executable_deepcli_actions(&preview_next_actions);
    assert!(preview_next_actions
        .iter()
        .any(|action| action == "deepcli fork --current"));
}

#[test]
fn fork_without_session_arg_defaults_to_current_session() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let current = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    current.append_message("user", "current context").unwrap();
    let other = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    other.append_message("user", "newer context").unwrap();

    let output = handle_fork(
        dir.path(),
        Some(current.id().to_string()),
        vec!["--no-open".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
        value["source"]["id"].as_str(),
        Some(current.id().to_string().as_str())
    );
}

#[test]
fn fork_without_current_prefers_resumable_context_over_diagnostic_activity() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let conversation = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    conversation
        .append_message("user", "continue compiler repair")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let diagnostic = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    diagnostic
        .append_tool_call(&ToolCallRecord {
            tool: "git_status".to_string(),
            input: json!({}),
            output: json!({"clean": true}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let output = handle_fork(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["source"]["id"], conversation.id().to_string());
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("resumable conversation context"));
}

#[test]
fn fork_current_without_active_session_reports_shell_fallbacks() {
    let dir = tempdir().unwrap();
    let error = handle_fork(
        dir.path(),
        None,
        vec!["--current".to_string(), "--dry-run".to_string()],
    )
    .unwrap_err();

    assert!(error.to_string().contains("omit `--current`"));
    assert!(error.to_string().contains("deepcli fork --dry-run --json"));
    assert!(error
        .to_string()
        .contains("deepcli resume candidates --json"));
    assert!(error
        .to_string()
        .contains("deepcli session list --all --limit 20 --json"));
}

#[test]
fn fork_current_json_without_active_session_returns_structured_error() {
    let dir = tempdir().unwrap();
    let error = handle_fork(
        dir.path(),
        None,
        vec![
            "--current".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/fork-error.json".to_string(),
        ],
    )
    .unwrap_err()
    .downcast::<CommandExit>()
    .unwrap();
    let value: Value = serde_json::from_str(&error.output).unwrap();
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/fork-error.json"))
        .expect("structured error should be written before non-zero exit");

    assert_eq!(error.code, 1);
    assert_eq!(written, error.output);
    assert_eq!(value["schema"], "deepcli.session.fork.v1");
    assert_eq!(value["status"], "error");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["source"], Value::Null);
    assert_eq!(value["fork"], Value::Null);
    assert_eq!(value["error"]["code"], "no_active_session");
    assert_eq!(value["nextActions"][0], "deepcli fork --dry-run --json");
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("deepcli resume candidates --json")));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| { action.as_str() == Some("deepcli session list --all --limit 20 --json") }));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Preview session fork".to_string()));
    assert!(checklist_labels.contains(&"Inspect resume candidates".to_string()));
}

#[test]
fn fork_without_resumable_context_reports_candidate_commands() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();

    let error = handle_fork(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("deepcli resume candidates --json"));
    assert!(error
        .to_string()
        .contains("deepcli session list --all --limit 20 --json"));
}

#[test]
fn fork_json_without_resumable_context_returns_structured_error() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    let tool_only = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    tool_only
        .append_tool_call(&ToolCallRecord {
            tool: "git_status".to_string(),
            input: json!({}),
            output: json!({"clean": true}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let error = handle_fork(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap_err()
    .downcast::<CommandExit>()
    .unwrap();
    let value: Value = serde_json::from_str(&error.output).unwrap();

    assert_eq!(error.code, 1);
    assert_eq!(value["schema"], "deepcli.session.fork.v1");
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "no_resumable_context");
    assert_eq!(value["terminal"]["wouldOpen"], true);
    assert!(value["report"].as_str().unwrap().contains("fork error"));
    assert_eq!(
        value["nextActions"][0],
        "deepcli session prune-empty --dry-run --json"
    );
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| { action.as_str() == Some("deepcli session diagnose --limit 5 --json") }));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("deepcli resume candidates --json")));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| { action.as_str() == Some("deepcli session list --all --limit 20 --json") }));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Preview empty session cleanup".to_string()));
    assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
    assert!(checklist_labels.contains(&"Inspect resume candidates".to_string()));
    assert!(checklist_labels.contains(&"List saved sessions".to_string()));
}

#[test]
fn fork_dry_run_json_previews_without_creating_session() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.rename("original task").unwrap();
    session.append_message("user", "hello").unwrap();
    session.set_state(SessionState::WaitingUser).unwrap();
    let before = store.list().unwrap().len();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.fork.v1");
    assert_eq!(value["status"], "dry_run");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["source"]["id"], session.id().to_string());
    assert!(value["fork"].is_null());
    assert_eq!(value["terminal"]["opened"], false);
    assert_eq!(value["terminal"]["resumeCommand"], Value::Null);
    assert_eq!(value["terminal"]["wouldOpen"], true);
    assert!(value["plannedFork"]["title"]
        .as_str()
        .unwrap()
        .contains("Fork of original task"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Create session fork".to_string()));
    assert!(next_actions
        .iter()
        .any(|action| action.starts_with("deepcli fork ")));
    assert!(value["report"].as_str().unwrap().contains("fork dry-run"));
    assert_eq!(store.list().unwrap().len(), before);
}

#[test]
fn fork_dry_run_json_preserves_custom_terminal_app() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.append_message("user", "continue here").unwrap();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--app".to_string(),
            "iTerm2".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["terminal"]["app"], "iTerm2");
    assert_eq!(value["terminal"]["wouldOpen"], true);
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli fork {} --app iTerm2", session.id())));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("terminal app: iTerm2"));
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn fork_dry_run_json_uses_terminal_app_env_default() {
    let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
    let _guard = EnvVarGuard::set("DEEPCLI_TERMINAL_APP", "iTerm2");
    let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.append_message("user", "continue here").unwrap();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["terminal"]["app"], "iTerm2");
    assert_eq!(value["terminal"]["autoResumeSupported"], true);
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli fork {} --app iTerm2", session.id())));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("terminal app: iTerm2"));
}

#[test]
fn fork_dry_run_json_infers_iterm_from_term_program_default() {
    let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
    let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
    let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "iTerm.app");
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("model".to_string()),
        )
        .unwrap();
    session.append_message("user", "continue here").unwrap();

    let output = handle_fork(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "--current".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["terminal"]["app"], "iTerm2");
    assert_eq!(value["terminal"]["autoResumeSupported"], true);
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli fork {} --app iTerm2", session.id())));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("terminal app: iTerm2"));
}

#[tokio::test]
async fn resume_dry_run_json_previews_session_without_starting_runtime() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("compiler recovery").unwrap();
    session.append_message("user", "repair compiler").unwrap();
    session
        .append_message("assistant", "plan next step")
        .unwrap();
    session.write_summary("resume summary").unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "read_file".to_string(),
            input: json!({"path": "src/main.rs"}),
            output: json!({"ok": true}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let before = store.list().unwrap().len();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let executor = test_executor(dir.path());
    let command = CommandRouter::parse(&format!(
        "/resume {} --dry-run --json --output .deepcli/exports/resume.json",
        session.id()
    ))
    .unwrap()
    .unwrap();

    let output = CommandRouter::handle(
        command,
        CommandContext {
            workspace: dir.path(),
            config: &config,
            registry: &registry,
            executor: &executor,
            session_id: None,
            provider_override: None,
            allow_interactive_prompts: true,
        },
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.resume.preview.v1");
    assert_eq!(value["status"], "preview");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["selected"]["id"], session.id().to_string());
    assert_eq!(value["selected"]["title"], "compiler recovery");
    assert_eq!(value["selected"]["activity"]["messages"], 2);
    assert_eq!(value["selected"]["activity"]["tools"], 1);
    assert_eq!(value["selected"]["hasSummary"], true);
    assert!(value["resumeCommand"]
        .as_str()
        .unwrap()
        .starts_with("deepcli resume "));
    assert!(value["recentMessages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["content"] == "repair compiler"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().starts_with("deepcli resume ")));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Resume saved work".to_string()));
    assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
    assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
    assert!(value["report"].as_str().unwrap().contains("resume preview"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/resume.json")).unwrap();
    assert_eq!(written, output);
    assert_eq!(store.list().unwrap().len(), before);
}

#[test]
fn resume_dry_run_json_without_resumable_context_returns_structured_error() {
    let dir = tempdir().unwrap();

    let error = handle_resume(
        dir.path(),
        None,
        vec![
            "--dry-run".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/resume-error.json".to_string(),
        ],
    )
    .unwrap_err()
    .downcast::<CommandExit>()
    .unwrap();

    let value: Value = serde_json::from_str(&error.output).unwrap();
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/resume-error.json")).unwrap();

    assert_eq!(error.code, 1);
    assert_eq!(written, error.output);
    assert_eq!(value["schema"], "deepcli.resume.preview.v1");
    assert_eq!(value["status"], "error");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["selected"], Value::Null);
    assert_eq!(value["error"]["code"], "no_resumable_context");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no session with resumable conversation context"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("deepcli sessions --all --limit 20")));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"List saved sessions".to_string()));
    assert!(value["report"].as_str().unwrap().contains("resume error"));
}

#[tokio::test]
async fn resume_candidates_json_explains_hidden_session_reasons() {
    let dir = tempdir().unwrap();
    let old_workspace = dir.path().with_file_name("old_deepcli");
    let store = SessionStore::new(dir.path());
    let eligible = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    eligible
        .append_message("user", "continue this compiler task")
        .unwrap();
    eligible.write_summary("continue compiler task").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    let tool_only = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    tool_only
        .append_tool_call(&ToolCallRecord {
            tool: "list_files".to_string(),
            input: json!({"path": "."}),
            output: json!({"files": ["src/main.rs"]}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    let mut low_information = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    low_information.append_message("user", "1").unwrap();
    let clarification =
        "我不确定你想执行什么。请说明要我分析、修改、测试、继续上次任务，或使用 /help 查看命令。";
    low_information
        .append_message("assistant", clarification)
        .unwrap();
    low_information.write_summary(clarification).unwrap();
    low_information
        .set_state(SessionState::WaitingUser)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    let old = store
        .create(
            &old_workspace,
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    old.append_message("user", "old workspace task").unwrap();
    old.write_summary("old workspace task").unwrap();

    let output = handle_resume(
        dir.path(),
        None,
        vec![
            "candidates".into(),
            "--json".into(),
            "--limit".into(),
            "10".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.resume.candidates.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["defaultCandidate"]["id"], eligible.id().to_string());
    assert_eq!(value["counts"]["total"], 4);
    assert_eq!(value["counts"]["eligible"], 1);
    assert_eq!(value["counts"]["hiddenToolOnly"], 1);
    assert_eq!(value["counts"]["hiddenLowInformation"], 1);
    assert_eq!(value["counts"]["hiddenOtherWorkspace"], 1);

    let candidates = value["candidates"].as_array().unwrap();
    assert!(candidates.iter().any(|candidate| {
        candidate["id"] == eligible.id().to_string()
            && candidate["eligible"] == true
            && candidate["hiddenReason"] == Value::Null
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate["id"] == tool_only.id().to_string()
            && candidate["eligible"] == false
            && candidate["hiddenReason"] == "tool_only_or_diagnostic"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate["id"] == low_information.id().to_string()
            && candidate["eligible"] == false
            && candidate["hiddenReason"] == "low_information_clarification"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate["id"] == old.id().to_string()
            && candidate["eligible"] == false
            && candidate["hiddenReason"] == "other_workspace"
    }));

    assert_eq!(
        value["nextActions"][0],
        format!(
            "deepcli resume {} --dry-run --json",
            short_id(&eligible.id())
        )
    );
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Resume preview".to_string()));
    assert!(checklist_labels.contains(&"List saved sessions".to_string()));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("hidden low-information sessions: 1"));
}

#[tokio::test]
async fn resume_candidates_without_eligible_sessions_recommends_empty_cleanup() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    let tool_only = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    tool_only
        .append_tool_call(&ToolCallRecord {
            tool: "list_files".to_string(),
            input: json!({"path": "."}),
            output: json!({"files": ["src/main.rs"]}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let output =
        handle_resume(dir.path(), None, vec!["candidates".into(), "--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.resume.candidates.v1");
    assert_eq!(value["defaultCandidate"], Value::Null);
    assert_eq!(value["counts"]["eligible"], 0);
    assert_eq!(value["counts"]["hiddenEmpty"], 1);
    assert_eq!(value["counts"]["hiddenToolOnly"], 1);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_eq!(
        next_actions[0],
        "deepcli session prune-empty --dry-run --json"
    );
    assert!(next_actions.contains(&"deepcli session list --all --limit 20 --json".to_string()));
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Preview empty session cleanup".to_string()));
    let candidates = value["candidates"].as_array().unwrap();
    assert!(candidates.iter().any(|candidate| {
        candidate["id"] == empty.id().to_string()
            && candidate["eligible"] == false
            && candidate["hiddenReason"] == "empty"
    }));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli session prune-empty --dry-run --json"));
}

#[tokio::test]
async fn resume_dry_run_without_id_skips_tool_only_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let conversation = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    conversation
        .append_message("user", "continue this compiler task")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let tool_only = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    tool_only
        .append_tool_call(&ToolCallRecord {
            tool: "list_files".to_string(),
            input: json!({"path": "."}),
            output: json!({"files": ["src/main.rs"]}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let test_only = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    test_only
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(101),
            stdout: String::new(),
            stderr: "failed".to_string(),
            passed: false,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let resumable = list_resumable_sessions(dir.path()).unwrap();
    assert_eq!(resumable.len(), 1);
    assert_eq!(resumable[0].id, conversation.id());

    let output = handle_resume(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["selected"]["id"], conversation.id().to_string());
    assert_eq!(value["selected"]["activity"]["messages"], 1);
    assert_ne!(value["selected"]["id"], tool_only.id().to_string());
    assert_ne!(value["selected"]["id"], test_only.id().to_string());
}

#[tokio::test]
async fn resume_dry_run_without_id_skips_low_information_clarification_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut conversation = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    conversation.rename("real compiler task").unwrap();
    conversation
        .append_message("user", "continue fixing the compiler parser")
        .unwrap();
    conversation
        .append_message("assistant", "I will inspect the parser failure")
        .unwrap();
    conversation
        .write_summary("Continue the compiler parser investigation")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    let clarification =
        "我不确定你想执行什么。请说明要我分析、修改、测试、继续上次任务，或使用 /help 查看命令。";
    let mut low_information = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    low_information.append_message("user", "1").unwrap();
    low_information
        .append_message("assistant", clarification)
        .unwrap();
    low_information.write_summary(clarification).unwrap();
    low_information
        .set_state(SessionState::WaitingUser)
        .unwrap();

    let resumable = list_resumable_sessions(dir.path()).unwrap();
    assert_eq!(resumable.len(), 1);
    assert_eq!(resumable[0].id, conversation.id());

    let output = handle_resume(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["selected"]["id"], conversation.id().to_string());
    assert_ne!(value["selected"]["id"], low_information.id().to_string());

    let explicit_output = handle_resume(
        dir.path(),
        None,
        vec![
            low_information.id().to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let explicit: Value = serde_json::from_str(&explicit_output).unwrap();
    assert_eq!(explicit["selected"]["id"], low_information.id().to_string());
}

#[tokio::test]
async fn resume_dry_run_without_id_skips_thin_completed_chat_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut conversation = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    conversation.rename("compiler task").unwrap();
    conversation
        .append_message("user", "continue implementing the compiler loop")
        .unwrap();
    conversation
        .append_message("assistant", "I will inspect the failing tests")
        .unwrap();
    conversation
        .write_summary("Continue implementing the compiler loop")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    let mut thin_completed = store
        .create(
            dir.path(),
            "kimi".to_string(),
            Some("kimi-for-coding".to_string()),
        )
        .unwrap();
    thin_completed
        .append_message(
            "user",
            "请用 read_file 读取 Cargo.toml 的前 20 行，然后用一句话说明项目名称。不要修改文件。",
        )
        .unwrap();
    thin_completed
            .append_message(
                "assistant",
                "这个项目名为 deepcli，是一个本地优先 AI 编码代理 CLI。\n\n[context cache] prompt_cache_hit_tokens=768 prompt_cache_miss_tokens=0 hit_rate=100.0%\n\n[usage estimate] prompt_tokens~233",
            )
            .unwrap();
    thin_completed
        .append_tool_call(&ToolCallRecord {
            tool: "read_file".to_string(),
            input: json!({"path": "Cargo.toml"}),
            output: json!({"content": "[package]\nname = \"deepcli\""}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    thin_completed
            .write_summary(
                "这个项目名为 deepcli，是一个本地优先 AI 编码代理 CLI。\n\n[context cache] prompt_cache_hit_tokens=768 prompt_cache_miss_tokens=0 hit_rate=100.0%\n\n[usage estimate] prompt_tokens~233",
            )
            .unwrap();
    thin_completed
        .save_plan(&Plan {
            title: "Plan for: read Cargo.toml".to_string(),
            steps: vec![PlanStep {
                id: "context".to_string(),
                description: "Read relevant workspace context.".to_string(),
                status: PlanStepStatus::Completed,
            }],
            updated_at: chrono::Utc::now(),
        })
        .unwrap();
    thin_completed.set_state(SessionState::Completed).unwrap();

    let resumable = list_resumable_sessions(dir.path()).unwrap();
    assert_eq!(resumable.len(), 1);
    assert_eq!(resumable[0].id, conversation.id());

    let output = handle_resume(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["selected"]["id"], conversation.id().to_string());
    assert_ne!(value["selected"]["id"], thin_completed.id().to_string());

    let explicit_output = handle_resume(
        dir.path(),
        None,
        vec![
            thin_completed.id().to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let explicit: Value = serde_json::from_str(&explicit_output).unwrap();
    assert_eq!(explicit["selected"]["id"], thin_completed.id().to_string());
}

#[tokio::test]
async fn resume_dry_run_without_id_prefers_current_workspace_sessions() {
    let dir = tempdir().unwrap();
    let old_workspace = dir.path().with_file_name("old_deepcli");
    let store = SessionStore::new(dir.path());
    let current = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    current
        .append_message("user", "continue the current workspace compiler task")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let old = store
        .create(
            &old_workspace,
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    old.append_message("user", "old workspace task").unwrap();
    old.write_summary("old workspace summary").unwrap();

    let resumable = list_resumable_sessions(dir.path()).unwrap();
    assert_eq!(resumable.len(), 1);
    assert_eq!(resumable[0].id, current.id());

    let list = handle_resume(dir.path(), None, Vec::new()).unwrap();
    assert!(list.contains(&current.id().to_string()[..8]));
    assert!(!list.contains(&old.id().to_string()[..8]));
    assert!(!list.contains("hidden non-resumable sessions"));

    let output = handle_resume(
        dir.path(),
        None,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["selected"]["id"], current.id().to_string());
    assert_ne!(value["selected"]["id"], old.id().to_string());

    let explicit_output = handle_resume(
        dir.path(),
        None,
        vec![
            old.id().to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let explicit: Value = serde_json::from_str(&explicit_output).unwrap();
    assert_eq!(explicit["selected"]["id"], old.id().to_string());
}

#[test]
fn terminal_dry_run_json_reports_command_without_opening_terminal() {
    let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
    let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
    let _term_guard = EnvVarGuard::remove("TERM_PROGRAM");
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let permissions = PermissionEngine::new(
        dir.path(),
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        dir.path(),
        permissions,
        None,
        config.agent.max_subagent_depth,
    );
    let output = handle_terminal(
        dir.path(),
        &executor,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.terminal.v1");
    assert_eq!(value["status"], "dry_run");
    assert_eq!(value["workspace"], dir.path().display().to_string());
    assert_eq!(value["command"], "open -a Terminal .");
    assert_eq!(value["opened"], false);
    let workspace_command = value["workspaceCommand"].as_str().unwrap();
    assert!(workspace_command.starts_with("cd "));
    assert!(workspace_command.contains(&dir.path().display().to_string()));
    assert_eq!(value["nextActions"][0], workspace_command);
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("workspace command: cd "));
}

#[test]
fn terminal_dry_run_json_supports_custom_terminal_app() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let permissions = PermissionEngine::new(
        dir.path(),
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        dir.path(),
        permissions,
        None,
        config.agent.max_subagent_depth,
    );
    let output = handle_terminal(
        dir.path(),
        &executor,
        vec![
            "--app".to_string(),
            "iTerm2".to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["schema"], "deepcli.terminal.v1");
    assert_eq!(value["app"], "iTerm2");
    assert_eq!(value["command"], "open -a iTerm2 .");
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli terminal --app iTerm2"));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli terminal --app iTerm2 --dry-run --json"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("terminal app: iTerm2"));
}

#[test]
fn terminal_dry_run_json_uses_terminal_app_env_default() {
    let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
    let _guard = EnvVarGuard::set("DEEPCLI_TERMINAL_APP", "iTerm2");
    let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "Apple_Terminal");
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let permissions = PermissionEngine::new(
        dir.path(),
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        dir.path(),
        permissions,
        None,
        config.agent.max_subagent_depth,
    );
    let output = handle_terminal(
        dir.path(),
        &executor,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["app"], "iTerm2");
    assert_eq!(value["command"], "open -a iTerm2 .");
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli terminal --app iTerm2"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("terminal app: iTerm2"));
}

#[test]
fn terminal_dry_run_json_infers_iterm_from_term_program_default() {
    let _lock = TERMINAL_ENV_LOCK.lock().unwrap();
    let _guard = EnvVarGuard::remove("DEEPCLI_TERMINAL_APP");
    let _term_guard = EnvVarGuard::set("TERM_PROGRAM", "iTerm.app");
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let permissions = PermissionEngine::new(
        dir.path(),
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        dir.path(),
        permissions,
        None,
        config.agent.max_subagent_depth,
    );
    let output = handle_terminal(
        dir.path(),
        &executor,
        vec!["--dry-run".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["app"], "iTerm2");
    assert_eq!(value["command"], "open -a iTerm2 .");
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli terminal --app iTerm2"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("terminal app: iTerm2"));
}

#[test]
fn terminal_opened_next_actions_are_still_executable() {
    let dir = tempdir().unwrap();
    let actions = terminal_next_actions(dir.path(), true, DEFAULT_TERMINAL_APP);
    assert!(!actions.is_empty(), "expected terminal next actions");
    for action in &actions {
        assert!(
            action.starts_with("deepcli ") || action.starts_with("cd "),
            "terminal next action should be directly executable: {action}"
        );
        assert!(
            !action.contains("use the opened terminal"),
            "terminal next action should not be prose: {action}"
        );
        assert!(
            !action.contains('<') && !action.contains('>'),
            "terminal next action should not contain placeholders: {action}"
        );
    }
    assert_eq!(actions[0], terminal_workspace_command(dir.path()));
    assert!(actions
        .iter()
        .any(|action| action == "deepcli terminal --dry-run --json"));
}

#[test]
fn help_contains_mvp_commands() {
    let help = CommandRouter::help_text();
    for command in CommandRouter::command_names() {
        assert!(help.contains(command), "{command} missing from help");
    }
    assert!(help.contains("/help <command>"));
    assert!(help.contains(
        "/verify [--run-tests|--test-command <command>] [--env-check [docker|compiler]]"
    ));
}

#[test]
fn command_specific_help_explains_usage_examples_and_notes() {
    let quickstart_help = CommandRouter::help_for(&["quickstart".to_string()]).unwrap();
    assert!(quickstart_help.contains("/quickstart - "));
    assert!(quickstart_help.contains("running-safe: no"));
    assert!(quickstart_help.contains("/quickstart --check"));
    assert!(quickstart_help.contains("/quickstart --json"));
    assert!(quickstart_help.contains("/quickstart --json --fail-on-missing"));
    assert!(quickstart_help.contains("exit non-zero"));
    assert!(quickstart_help.contains("provider turn timeout"));
    assert!(quickstart_help.contains("self-contained"));
    assert!(quickstart_help.contains("deepcli credentials set deepseek"));
    assert!(quickstart_help.contains("/model set deepseek deepseek-v4-pro"));
    assert!(quickstart_help.contains("deepcli accept --json"));
    assert!(quickstart_help.contains("deepcli gate --json"));
    assert!(quickstart_help.contains("/accept --env-check compiler --json"));

    let recipes_help = CommandRouter::help_for(&["recipes".to_string()]).unwrap();
    assert!(recipes_help.contains("/recipes - "));
    assert!(recipes_help.contains("running-safe: yes"));
    assert!(recipes_help.contains("deepcli.recipes.v1"));
    assert!(recipes_help.contains("Supported topics"));
    assert!(recipes_help.contains("deepcli recipes release --json"));
    let scorecard_help = CommandRouter::help_for(&["scorecard".to_string()]).unwrap();
    assert!(scorecard_help.contains("/scorecard - "));
    assert!(scorecard_help.contains("running-safe: yes"));
    assert!(scorecard_help.contains("deepcli.scorecard.v1"));
    assert!(scorecard_help.contains("When gaps exist"));
    assert!(scorecard_help.contains("sustained product loop"));
    assert!(scorecard_help.contains("deepcli benchmark --fail-below 85"));

    let opportunities_help = CommandRouter::help_for(&["opportunities".to_string()]).unwrap();
    assert!(opportunities_help.contains("/opportunities - "));
    assert!(opportunities_help.contains("running-safe: yes"));
    assert!(opportunities_help.contains("deepcli.opportunities.v1"));
    assert!(opportunities_help.contains("deepcli opportunities --json"));

    let bench_help = CommandRouter::help_for(&["benchmark".to_string()]).unwrap();
    assert!(bench_help.contains("/benchmark - "));
    assert!(bench_help.contains("deepcli.benchmark.record.v1"));
    assert!(bench_help.contains("deepcli.benchmark.suite.v1"));
    assert!(bench_help.contains("deepcli.benchmark.status.v1"));
    assert!(bench_help.contains("deepcli.benchmark.summary.v1"));
    assert!(bench_help.contains("deepcli.benchmark.trends.v1"));
    assert!(bench_help.contains("deepcli.benchmark.baseline.v1"));
    assert!(bench_help.contains("deepcli.benchmark.compare.v1"));
    assert!(bench_help.contains("deepcli.benchmark.baselines.v1"));
    assert!(bench_help.contains("deepcli.benchmark.cleanup.v1"));
    assert!(bench_help.contains("/benchmark presets"));
    assert!(bench_help.contains("/benchmark run-suite"));
    assert!(bench_help.contains("--preset <name>"));
    assert!(bench_help.contains("/benchmark record"));
    assert!(bench_help.contains("/benchmark status"));
    assert!(bench_help.contains("/benchmark gate"));
    assert!(bench_help.contains("--fail-on-not-ready"));
    assert!(bench_help.contains("non-zero exit"));
    assert!(bench_help.contains("/benchmark summary"));
    assert!(bench_help.contains("/benchmark trends"));
    assert!(bench_help.contains("/benchmark baseline-template"));
    assert!(bench_help.contains("/benchmark compare"));
    assert!(bench_help.contains("/benchmark baselines"));
    assert!(bench_help.contains("/benchmark clean"));
    assert!(bench_help.contains("--keep n"));

    let round_help = CommandRouter::help_for(&["round".to_string()]).unwrap();
    assert!(round_help.contains("/round - "));
    assert!(round_help.contains("running-safe: yes"));
    assert!(round_help.contains("deepcli.round.v1"));
    assert!(round_help.contains("--fail-on-gaps"));
    assert!(round_help.contains("--run-benchmark"));
    assert!(round_help.contains("--fail-on-command"));
    assert!(round_help.contains("goalStatus"));
    assert!(round_help.contains("failing gate remediation"));
    assert!(round_help.contains("skips the redundant `deepcli scorecard --json` action"));
    assert!(round_help.contains("deepcli round --json"));

    let selftest_help = CommandRouter::help_for(&["selftest".to_string()]).unwrap();
    assert!(selftest_help.contains("/selftest - "));
    assert!(selftest_help.contains("running-safe: yes"));
    assert!(selftest_help.contains("deepcli.selftest.v1"));
    assert!(selftest_help.contains("does not create a session or call a provider"));
    assert!(selftest_help.contains("deepcli selftest --json --fail-on-issues"));

    let preflight_help = CommandRouter::help_for(&["preflight".to_string()]).unwrap();
    assert!(preflight_help.contains("/preflight - "));
    assert!(preflight_help.contains("running-safe: yes"));
    assert!(preflight_help.contains("deepcli.preflight.v1"));
    assert!(preflight_help.contains("slowest check"));
    assert!(preflight_help.contains("does not create a session or call a provider"));
    assert!(preflight_help.contains("deepcli preflight --json"));
    assert!(preflight_help.contains("deepcli release-check --dry-run"));

    let completion_help = CommandRouter::help_for(&["completion".to_string()]).unwrap();
    assert!(completion_help.contains("/completion - "));
    assert!(completion_help.contains("running-safe: yes"));
    assert!(completion_help.contains("deepcli.completion.v1"));
    assert!(completion_help.contains("deepcli.completion.install.v1"));
    assert!(completion_help.contains("deepcli.completion.status.v1"));
    assert!(completion_help.contains("deepcli completion status zsh"));
    assert!(completion_help.contains("deepcli completion install zsh --force"));
    assert!(completion_help.contains("deepcli completion zsh"));
    assert!(completion_help.contains("does not create a session or call a provider"));

    let version_help = CommandRouter::help_for(&["version".to_string()]).unwrap();
    assert!(version_help.contains("/version - "));
    assert!(version_help.contains("running-safe: no"));
    assert!(version_help.contains("deepcli.version.v1"));
    assert!(version_help.contains("project config presence"));
    assert!(version_help.contains("without creating a session or calling a provider"));
    let compiler_help = CommandRouter::help_for(&["compiler".to_string()]).unwrap();
    assert!(compiler_help.contains("/compiler - "));
    assert!(compiler_help.contains("/compiler check"));
    assert!(compiler_help.contains("/compiler setup --smoke"));
    assert!(compiler_help.contains("running-safe: no"));

    let install_help = CommandRouter::help_for(&["install".to_string()]).unwrap();
    assert!(install_help.contains("/install - "));
    assert!(install_help.contains("deepcli install compiler --smoke"));
    assert!(install_help.contains("/compiler install"));

    let usage_help = CommandRouter::help_for(&["usage".to_string()]).unwrap();
    assert!(usage_help.contains("running-safe: yes"));
    assert!(usage_help.contains("/usage --json"));
    assert!(usage_help.contains("/usage --output"));
    assert!(usage_help.contains("deepcli.usage.v1"));

    let status_help = CommandRouter::help_for(&["status".to_string()]).unwrap();
    assert!(status_help.contains("/status --json"));
    assert!(status_help.contains("/status --output"));
    assert!(status_help.contains("deepcli.status.v1"));
    assert!(status_help.contains("running-safe: yes"));

    let privacy_help = CommandRouter::help_for(&["privacy".to_string()]).unwrap();
    assert!(privacy_help.contains("/privacy - "));
    assert!(privacy_help.contains("running-safe: yes"));
    assert!(privacy_help.contains("deepcli.privacy.scan.v1"));
    assert!(privacy_help.contains("does not create a session or call a provider"));
    assert!(privacy_help.contains("deepcli privacy --json"));

    let diagnose_help = CommandRouter::help_for(&["diagnose".to_string()]).unwrap();
    assert!(diagnose_help.contains("/diagnose [docker|compiler]"));
    assert!(diagnose_help.contains("/diagnose docker --json"));
    assert!(diagnose_help.contains("run an environment check for `<target>`"));
    assert!(diagnose_help.contains("/diagnose --full-env"));
    assert!(diagnose_help.contains("/diagnose --json"));
    assert!(diagnose_help.contains("/diagnose --output"));
    assert!(diagnose_help.contains("/diagnose --bundle"));
    assert!(diagnose_help.contains("redacted support bundle"));
    assert!(diagnose_help.contains("workspace health check"));
    assert!(diagnose_help.contains("workspace-contained file"));
    assert!(diagnose_help.contains("running-safe: no"));

    let support_help = CommandRouter::help_for(&["support".to_string()]).unwrap();
    assert!(support_help.contains("/support - "));
    assert!(support_help.contains("deepcli support"));
    assert!(support_help.contains(DEFAULT_SUPPORT_BUNDLE_DIR));
    assert!(support_help.contains("issue.md"));
    assert!(support_help.contains("version.json"));
    assert!(support_help.contains("logs.json"));
    assert!(support_help.contains("shortcut for `/diagnose --bundle`"));

    let doctor_help = CommandRouter::help_for(&["doctor".to_string()]).unwrap();
    assert!(doctor_help.contains("/doctor shell"));
    assert!(doctor_help.contains("/doctor shell --json"));
    assert!(doctor_help.contains("/doctor [docker|compiler]"));
    assert!(doctor_help.contains("/doctor docker --json"));
    assert!(doctor_help.contains("run an environment check for `<target>`"));
    assert!(doctor_help.contains("resolves to this workspace"));
    assert!(doctor_help.contains("shell completion state"));
    assert!(doctor_help.contains("/doctor --json"));
    assert!(doctor_help.contains("/doctor --output"));
    assert!(doctor_help.contains("deepcli.doctor.v1"));
    assert!(doctor_help.contains("deepcli version"));
    assert!(doctor_help.contains("provider turn timeout"));
    assert!(doctor_help.contains("workspace-contained file"));

    let trace_help = CommandRouter::help_for(&["trace".to_string()]).unwrap();
    assert!(trace_help.contains("/trace --json"));
    assert!(trace_help.contains("/trace --output"));
    assert!(trace_help.contains("deepcli.trace.v1"));
    assert!(trace_help.contains("redacted"));
    assert!(trace_help.contains("running-safe: yes"));

    let logs_help = CommandRouter::help_for(&["logs".to_string()]).unwrap();
    assert!(logs_help.contains("/logs --list"));
    assert!(logs_help.contains("/logs --file <log-file>"));
    assert!(logs_help.contains("deepcli.logs.v1"));
    assert!(logs_help.contains("running-safe: yes"));

    let terminal_help = CommandRouter::help_for(&["terminal".to_string()]).unwrap();
    assert!(terminal_help.contains("running-safe: yes"));
    assert!(terminal_help.contains("/terminal --dry-run --json"));
    assert!(terminal_help.contains("workspaceCommand"));

    let cmd_help = CommandRouter::help_for(&["cmd".to_string()]).unwrap();
    assert!(cmd_help.contains("running-safe: no"));
    assert!(cmd_help.contains("/cmd <bash command>"));
    assert!(cmd_help.contains("/cmd --attach <bash command>"));
    assert!(cmd_help.contains("/cmd git status --short"));
    assert!(cmd_help.contains("Plain `/cmd` is local-only and does not call a provider"));
    assert!(cmd_help.contains("/cmd --attach"));

    let permissions_help = CommandRouter::help_for(&["permissions".to_string()]).unwrap();
    assert!(permissions_help.contains("/permissions [show] [--json] [--output path]"));
    assert!(permissions_help.contains("deepcli.permissions.show.v1"));
    assert!(permissions_help.contains("workspace-contained file"));

    let login_help = CommandRouter::help_for(&["login".to_string()]).unwrap();
    assert!(login_help.contains("/login - "));
    assert!(login_help.contains("/credentials set"));
    assert!(login_help.contains("deepcli login deepseek --stdin"));
    assert!(login_help.contains("should not create a session or call a provider"));
    assert!(login_help.contains("running-safe: no"));

    let apikey_help = CommandRouter::help_for(&["apikey".to_string()]).unwrap();
    assert!(apikey_help.contains("no provider call"));

    let logout_help = CommandRouter::help_for(&["logout".to_string()]).unwrap();
    assert!(logout_help.contains("/logout [provider]"));
    assert!(logout_help.contains("/credentials remove"));
    assert!(logout_help.contains("does not create a session or call a provider"));

    let timeout_help = CommandRouter::help_for(&["timeout".to_string()]).unwrap();
    assert!(timeout_help.contains("/timeout [show|set <seconds>|reset]"));
    assert!(timeout_help.contains("agent.providerTurnTimeoutSeconds"));
    assert!(timeout_help.contains("should not create an empty session"));

    let model_help = CommandRouter::help_for(&["model".to_string()]).unwrap();
    assert!(model_help.contains("/model show [--json] [--output path]"));
    assert!(model_help.contains("/model list [--json] [--output path]"));
    assert!(model_help.contains("/model <provider> [model]"));
    assert!(model_help.contains("run locally without creating an empty session"));
    assert!(model_help.contains("deepcli.model.inspect.v1"));
    assert!(model_help.contains("workspace-contained file"));

    let git_help = CommandRouter::help_for(&["git".to_string()]).unwrap();
    assert!(git_help.contains("/git status --json"));
    assert!(git_help.contains("/git diff --staged --json"));
    assert!(git_help.contains("--output .deepcli/exports/git-status.json"));
    assert!(git_help.contains("deepcli.git.inspect.v1"));
    assert!(git_help.contains("running-safe: yes"));

    let goal_help = CommandRouter::help_for(&["goal".to_string()]).unwrap();
    assert!(goal_help.contains("/goal <objective>"));
    assert!(goal_help.contains("/goal pause"));
    assert!(goal_help.contains("/goal resume"));
    assert!(goal_help.contains("/goal complete"));
    assert!(goal_help.contains("--token-budget"));
    assert!(goal_help.contains("/goal status"));
    assert!(goal_help.contains("/goal gate"));
    assert!(goal_help.contains("deepcli.goal.status.v1"));

    let plan_help = CommandRouter::help_for(&["plan".to_string()]).unwrap();
    assert!(plan_help.contains("/plan <rough requirement>"));
    assert!(plan_help.contains("active provider"));
    assert!(plan_help.contains("ask_user_question"));

    let fork_help = CommandRouter::help_for(&["fork".to_string()]).unwrap();
    assert!(fork_help.contains("/fork --current"));
    assert!(fork_help.contains("/fork --current --dry-run --json"));
    assert!(fork_help.contains("/fork --current --no-open --verify --json"));
    assert!(fork_help.contains("/fork <session_id>"));
    assert!(fork_help.contains("deepcli resume <new_id>"));
    assert!(fork_help.contains("verification"));
    assert!(fork_help.contains("without creating a session"));
    assert!(fork_help.contains("skip Terminal launch"));
    assert!(fork_help.contains("running-safe: yes"));

    let resume_help = CommandRouter::help_for(&["resume".to_string()]).unwrap();
    assert!(resume_help.contains("/resume <session_id> --dry-run --json"));
    assert!(resume_help.contains("deepcli.resume.preview.v1"));
    assert!(resume_help.contains("does not start interactive chat"));
    assert!(resume_help.contains("workspace-contained output"));

    let stop_help = CommandRouter::help_for(&["stop".to_string()]).unwrap();
    assert!(stop_help.contains("/stop - "));
    assert!(stop_help.contains("running-safe: yes"));

    let slash_help = CommandRouter::help_for(&["/credentials".to_string()]).unwrap();
    assert!(slash_help.contains("/credentials status [provider] [--json] [--output path]"));
    assert!(slash_help.contains("deepcli.credentials.status.v1"));
    assert!(slash_help.contains("workspace-contained file"));
    assert!(slash_help.contains("Plaintext API keys are redacted"));

    let alias_help = CommandRouter::help_for(&["quit".to_string()]).unwrap();
    assert!(alias_help.contains("/quit - "));

    let init_help = CommandRouter::help_for(&["init".to_string()]).unwrap();
    assert!(init_help.contains("/init --probe-provider"));
    assert!(init_help.contains("low-risk local scaffolding"));

    let config_help = CommandRouter::help_for(&["config".to_string()]).unwrap();
    assert!(config_help.contains("/config show [--json] [--output path]"));
    assert!(config_help.contains("/config get <path> [--json] [--output path]"));
    assert!(config_help.contains("deepcli.config.inspect.v1"));
    assert!(config_help.contains("workspace-contained file"));

    let prompt_help = CommandRouter::help_for(&["prompt".to_string()]).unwrap();
    assert!(prompt_help.contains("/prompt list [--json] [--output path]"));
    assert!(prompt_help.contains("/prompt get <name> [--json] [--output path]"));
    assert!(prompt_help.contains("deepcli.prompt.inspect.v1"));
    assert!(prompt_help.contains("workspace-contained file"));
    assert!(prompt_help.contains("/prompt delete <name>"));
    assert!(prompt_help.contains("override built-in prompt names"));

    let skill_help = CommandRouter::help_for(&["skill".to_string()]).unwrap();
    assert!(skill_help.contains("/skill list [--json] [--output path]"));
    assert!(skill_help.contains("/skill run <name> [--json] [--output path]"));
    assert!(skill_help.contains("deepcli.skill.inspect.v1"));
    assert!(skill_help.contains("workspace-contained file"));

    let agent_help = CommandRouter::help_for(&["agent".to_string()]).unwrap();
    assert!(agent_help.contains("/agent list [--json] [--output path]"));
    assert!(agent_help.contains("/agent show <id> [--json] [--output path]"));
    assert!(!agent_help.contains("/agent run <id>"));
    assert!(agent_help.contains("/agent resume <id> [--json] [--output path]"));
    assert!(agent_help.contains("/agent logs <id> [--json] [--output path]"));
    assert!(agent_help.contains("deepcli.agent.inspect.v1"));
    assert!(agent_help.contains("real child `AgentRuntime`"));
    assert!(agent_help.contains("workspace-contained file"));

    let test_help = CommandRouter::help_for(&["test".to_string()]).unwrap();
    assert!(test_help.contains("/test [discover] [--json] [--output path]"));
    assert!(test_help.contains("/test run [--json] [--output path] -- <command>"));
    assert!(test_help.contains("deepcli.test.inspect.v1"));
    assert!(test_help.contains("workspace-contained file"));

    let web_help = CommandRouter::help_for(&["web".to_string()]).unwrap();
    assert!(web_help.contains("/web search <query>"));

    let approval_help = CommandRouter::help_for(&["approval".to_string()]).unwrap();
    assert!(approval_help.contains("/approval list [--json] [--output path]"));
    assert!(approval_help.contains("deepcli.approval.list.v1"));
    assert!(approval_help.contains("deepcli.approval.action.v1"));
    assert!(approval_help.contains("/approval approve <id> [--current] [--json] [--output path]"));
    assert!(approval_help.contains("workspace-contained file"));

    let btw_help = CommandRouter::help_for(&["btw".to_string()]).unwrap();
    assert!(btw_help.contains("/btw list [--json] [--output path]"));
    assert!(btw_help.contains("deepcli.btw.list.v1"));
    assert!(btw_help.contains("deepcli.btw.action.v1"));
    assert!(btw_help.contains("/btw answer <id> [--current] [--json] [--output path] <answer>"));
    assert!(btw_help.contains("workspace-contained file"));

    let session_help = CommandRouter::help_for(&["session".to_string()]).unwrap();
    assert!(session_help.contains("/session list [--all] [--limit n] [--json]"));
    assert!(session_help.contains("/session search <query> [--limit n] [--json]"));
    assert!(session_help.contains("deepcli.session.list.v1"));
    assert!(session_help.contains("deepcli.session.search.v1"));
    assert!(session_help.contains("/session next [--json] [--output path]"));
    assert!(session_help.contains("deepcli.next.v1"));
    assert!(session_help.contains("/session diagnose [--limit n] [--json] [--output path]"));
    assert!(session_help.contains("deepcli.session.diagnose.v1"));
    assert!(session_help.contains("/session history [--limit n] [--json] [--output path]"));
    assert!(session_help.contains("/session tools [--failed] [--limit n] [--json]"));
    assert!(session_help.contains("deepcli.session.inspect.v1"));
    assert!(session_help.contains("signal counts"));
    assert!(session_help.contains("/session rename <session_id|--current> <title>"));
    assert!(
        session_help.contains("/session prune-empty [--dry-run|--force] [--json] [--output path]")
    );
    assert!(session_help.contains("deepcli.session.prune_empty.v1"));
    assert!(session_help.contains("/session tools [--failed] [--limit n]"));
    assert!(session_help.contains("/session diffs [--limit n]"));
    assert!(session_help.contains("/session backups [--limit n]"));

    let cleanup_help = CommandRouter::help_for(&["cleanup".to_string()]).unwrap();
    assert!(cleanup_help.contains("/cleanup - "));
    assert!(cleanup_help.contains("/session prune-empty"));
    assert!(cleanup_help.contains("deepcli.session.prune_empty.v1"));
    assert!(cleanup_help.contains("running-safe: yes"));

    let accept_help = CommandRouter::help_for(&["accept".to_string()]).unwrap();
    assert!(accept_help.contains("/accept - "));
    assert!(accept_help.contains("running-safe: no"));
    assert!(accept_help.contains("/verify --run-tests"));
    assert!(accept_help.contains("deepcli.verify.v1"));
    assert!(accept_help.contains("/gate"));

    let gate_help = CommandRouter::help_for(&["gate".to_string()]).unwrap();
    assert!(gate_help.contains("/gate - "));
    assert!(gate_help.contains("running-safe: no"));
    assert!(gate_help.contains("/verify --run-tests --fail-on-blockers"));
    assert!(gate_help.contains("non-zero exit"));

    let verify_help = CommandRouter::help_for(&["verify".to_string()]).unwrap();
    assert!(verify_help.contains("/verify --limit <n>"));
    assert!(verify_help.contains("/verify --run-tests"));
    assert!(verify_help.contains("/verify --test-command 'cargo test'"));
    assert!(verify_help.contains("/verify --env-check [docker|compiler]"));
    assert!(verify_help.contains("/verify --json"));
    assert!(verify_help.contains("/verify --output"));
    assert!(verify_help.contains("/verify --fail-on-blockers"));
    assert!(verify_help.contains("acceptance report"));
    assert!(verify_help.contains("environment readiness"));
    assert!(verify_help.contains("machine-readable"));
    assert!(verify_help.contains("workspace-contained file"));
    assert!(verify_help.contains("exit non-zero"));

    let handoff_help = CommandRouter::help_for(&["handoff".to_string()]).unwrap();
    assert!(handoff_help.contains("/handoff --markdown"));
    assert!(handoff_help.contains("/handoff --pr"));
    assert!(handoff_help.contains("/handoff --env-check [docker|compiler]"));
    assert!(handoff_help.contains("/handoff --json"));
    assert!(handoff_help.contains("/handoff --fail-on-blockers"));
    assert!(handoff_help.contains("/handoff --output"));
    assert!(handoff_help.contains("pull-request description template"));
    assert!(handoff_help.contains("environment readiness"));
    assert!(handoff_help.contains("workspace-contained file"));
    assert!(handoff_help.contains("exit non-zero"));
}

#[test]
fn help_all_and_unknown_topics_are_handled() {
    let all = CommandRouter::help_for(&["all".to_string()]).unwrap();
    assert!(all.contains("/quickstart - "));
    assert!(all.contains("/compiler - "));
    assert!(all.contains("/session - "));
    assert!(all.contains("/diagnose - "));
    assert!(all.contains("/doctor - "));

    let error = CommandRouter::help_for(&["missing".to_string()])
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown help topic `missing`"));
}

#[test]
fn quickstart_check_json_output_is_contextual_and_written() {
    let dir = tempdir().unwrap();
    let config = test_provider_config(MISSING_TEST_PROVIDER);
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"quickstart-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_quickstart(
        dir.path(),
        &config,
        &executor,
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/quickstart.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.quickstart.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["version"]["package"], "deepcli");
    assert_eq!(value["version"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(value["version"]["commandCount"].as_u64().unwrap() > 0);
    assert_eq!(value["config"]["providerTurnTimeoutSeconds"], 600);
    assert_eq!(value["readiness"]["ready"], false);
    assert!(value["readiness"]["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("project config")));
    assert!(value["readiness"]["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("provider API key")));
    assert_eq!(value["projectConfig"]["present"], false);
    assert_eq!(value["provider"]["name"], MISSING_TEST_PROVIDER);
    assert_eq!(value["provider"]["apiKey"], "missing");
    assert_eq!(value["tests"]["count"], 1);
    assert!(value["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("deepcli")));
    assert!(value["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("/recipes")));
    assert!(value["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("/scorecard --json")));
    assert!(value["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("/accept --json")));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli credentials set missing-provider-2f7c1e"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli accept --json"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli recipes"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli scorecard --json"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli gate --json"));
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(
        next_actions.iter().all(|item| {
            let action = item.as_str().unwrap();
            action.starts_with("deepcli ")
                || action.starts_with("cargo ")
                || action.starts_with("git ")
        }),
        "quickstart JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    assert!(
            next_actions.iter().all(|item| {
                let action = item.as_str().unwrap();
                !action.contains("`/") && !action.starts_with("run `")
            }),
            "quickstart JSON nextActions should not require parsing slash-command prose: {next_actions:?}"
        );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains(concat!("version: ", env!("CARGO_PKG_VERSION"))));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("registered slash commands:"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("provider turn timeout: 600s"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("recommended flow:"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/quickstart.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn quickstart_fail_on_missing_returns_report_and_writes_output() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_quickstart(
        dir.path(),
        &AppConfig::default(),
        &executor,
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/quickstart-gate.json".into(),
            "--fail-on-missing".into(),
        ],
    )
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);

    let value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(value["schema"], "deepcli.quickstart.v1");
    assert_eq!(value["readiness"]["ready"], false);
    assert!(value["readiness"]["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("project config")));
    assert!(value["readiness"]["missing"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("project tests")));

    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/quickstart-gate.json")).unwrap();
    assert_eq!(written, exit.output);
}

#[test]
fn quickstart_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_quickstart(
        dir.path(),
        &AppConfig::default(),
        &executor,
        vec!["--output".into(), "../quickstart.txt".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../quickstart.txt").exists());
}

#[test]
fn recipes_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec![
            "release".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/recipes.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.recipes.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["topic"], "release");
    assert_eq!(value["recipes"].as_array().unwrap().len(), 1);
    assert_eq!(value["recipes"][0]["name"], "release");
    assert!(value["recipes"][0]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command.as_str().unwrap() == "deepcli preflight --json"));
    assert!(value["availableTopics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|topic| topic.as_str().unwrap() == "debug"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli preflight --json"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|action| action.as_str().unwrap().starts_with("deepcli")));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli recipes"));
    assert!(!dir.path().join(".deepcli/sessions").exists());

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/recipes.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn recipes_aliases_topics_and_output_safety_are_enforced() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["ship".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["topic"], "release");

    let unknown = handle_recipes(dir.path(), &config, &registry, vec!["unknown".into()])
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("unknown /recipes topic `unknown`"));

    let traversal = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["--output".into(), "../recipes.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../recipes.json").exists());
}

#[test]
fn recipes_sota_topic_guides_product_loop_and_benchmark_compare() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["product-loop".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.recipes.v1");
    assert_eq!(value["topic"], "sota");
    assert_eq!(value["title"], "SOTA Product Loop");
    assert!(value["summary"]
        .as_str()
        .unwrap()
        .contains("Inspect product gaps"));
    let next_actions = value["nextActions"].as_array().unwrap();
    let checklist = value["checklist"].as_array().unwrap();
    assert_eq!(checklist.len(), next_actions.len());
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"], index + 1);
        assert_eq!(item["command"], next_actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
    assert_eq!(
        checklist[0]["command"],
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert_eq!(checklist[0]["label"], "Refresh benchmark evidence");
    assert!(checklist
        .iter()
        .all(|item| { item["command"].as_str().unwrap().starts_with("deepcli ") }));
    assert_eq!(value["recipes"].as_array().unwrap().len(), 1);
    assert_eq!(value["recipes"][0]["name"], "sota");
    let commands = value["recipes"][0]["commands"].as_array().unwrap();
    assert!(commands
        .iter()
        .any(|command| command.as_str().unwrap() == "deepcli round --json"));
    assert!(commands.iter().any(|command| {
        command
            .as_str()
            .unwrap()
            .contains("round --json --run-benchmark --fail-on-command")
    }));
    assert!(commands.iter().any(|command| {
        command
            .as_str()
            .unwrap()
            .contains("benchmark baseline-template --output")
    }));
    assert!(commands.iter().any(|command| {
        command
            .as_str()
            .unwrap()
            .contains("benchmark compare --baseline")
    }));
    assert!(value["availableTopics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|topic| topic.as_str().unwrap() == "sota"));
    assert_eq!(
        next_actions.first().unwrap().as_str().unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert!(next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
    assert!(!next_actions.iter().any(|action| {
        action.as_str().unwrap()
            == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
    }));
    assert!(next_actions
        .iter()
        .all(|action| action.as_str().unwrap().starts_with("deepcli")));
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap().contains("run `/")));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("sota - SOTA Product Loop"));

    let trend_dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(trend_dir.path());
    let now = Utc::now();
    for preset in MEANINGFUL_BENCHMARK_PRESETS {
        write_benchmark_status_test_artifact(
            trend_dir.path(),
            &format!("20990101T000000Z-product-{preset}.json"),
            now,
            preset,
            preset,
            "passed",
        );
    }
    let trend_output = handle_recipes(
        trend_dir.path(),
        &config,
        &registry,
        vec!["sota".into(), "--json".into()],
    )
    .unwrap();
    let trend_value: Value = serde_json::from_str(&trend_output).unwrap();
    let trend_next_actions = trend_value["nextActions"].as_array().unwrap();
    assert_eq!(
        trend_next_actions.first().unwrap().as_str().unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );

    let help = CommandRouter::help_for(&["recipes".to_string()]).unwrap();
    assert!(help.contains("/recipes sota"));
    assert!(help.contains("product-loop"));
}

#[test]
fn recipes_sota_next_actions_compare_when_default_baseline_exists() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_ready_competitor_baseline(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["sota".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"].as_array().unwrap();

    assert!(next_actions.iter().any(|action| {
        action.as_str().unwrap()
            == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
    }));
    assert!(!next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
}

#[test]
fn recipes_sota_checklist_matches_baseline_state_when_current_capture_is_ready() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["sota".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action.as_str().unwrap())
        .collect::<Vec<_>>();
    let checklist = value["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["command"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(checklist, next_actions);
    assert!(!checklist.contains(&"deepcli recipes sota --json"));
    assert!(checklist.contains(
            &"deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        ));
    assert!(checklist.contains(
        &"deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
    ));
    assert!(!checklist.contains(
        &"deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
    ));
}

#[test]
fn recipes_sota_surfaces_ready_round_product_opportunities() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["sota".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["topic"], "sota");
    let opportunities = value["opportunities"].as_array().unwrap();
    assert_eq!(value["opportunityEffortCounts"]["medium"], 1);
    assert_eq!(value["opportunityEffortCounts"]["low"], 1);
    let baseline_opportunity = opportunities
        .iter()
        .find(|opportunity| opportunity["id"] == "competitor_baseline")
        .expect("SOTA recipe should explain the baseline opportunity");
    assert_eq!(baseline_opportunity["status"], "available");
    assert_eq!(baseline_opportunity["priority"], "high");
    assert!(baseline_opportunity["impact"]
        .as_str()
        .unwrap()
        .contains("benchmark"));
    assert_eq!(
        baseline_opportunity["checklist"][0]["command"],
        baseline_opportunity["nextActions"][0]
    );
}

#[test]
fn opportunities_json_lists_current_round_product_opportunities() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output =
        handle_opportunities(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.opportunities.v1");
    assert_eq!(value["status"], "ready");
    assert_eq!(value["ready"], true);
    assert_eq!(value["source"]["command"], "deepcli round --json");
    assert!(value["opportunityCount"].as_u64().unwrap() >= 2);
    let opportunities = value["opportunities"].as_array().unwrap();
    assert_eq!(value["summary"]["status"], value["status"]);
    assert_eq!(value["summary"]["ready"], value["ready"]);
    assert_eq!(value["summary"]["priorityFilter"], Value::Null);
    assert_eq!(value["summary"]["effortFilter"], Value::Null);
    assert_eq!(value["summary"]["opportunityCount"], opportunities.len());
    assert_eq!(
        value["summary"]["totalOpportunityCount"],
        value["totalOpportunityCount"]
    );
    assert_eq!(value["summary"]["filteredOutOpportunityCount"], 0);
    assert_eq!(
        value["recommendedOpportunity"]["id"],
        opportunities[0]["id"]
    );
    assert_eq!(
        value["summary"]["recommendedOpportunityId"],
        value["recommendedOpportunity"]["id"]
    );
    assert_eq!(
        value["recommendedOpportunity"]["checklist"][0]["command"],
        opportunities[0]["nextActions"][0]
    );
    assert_eq!(value["opportunityPriorityCounts"]["high"], 1);
    assert_eq!(value["opportunityPriorityCounts"]["medium"], 1);
    let baseline_opportunity = opportunities
        .iter()
        .find(|opportunity| opportunity["id"] == "competitor_baseline")
        .expect("opportunities should include the competitor baseline workflow");
    assert_eq!(baseline_opportunity["priority"], "high");
    assert_eq!(baseline_opportunity["effort"], "medium");
    assert_eq!(
        baseline_opportunity["nextActions"][0],
        "deepcli benchmark baselines --json"
    );
    assert_eq!(
        baseline_opportunity["checklist"][0]["command"],
        "deepcli benchmark baselines --json"
    );
    let next_actions = json_string_array(&value["nextActions"]);
    assert_eq!(next_actions[0], "deepcli benchmark baselines --json");
    assert!(next_actions.iter().any(|action| {
            action == "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        }));
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["checklist"][0]["command"]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );

    let text = handle_opportunities(dir.path(), &config, &registry, Vec::new()).unwrap();
    assert!(text.contains("recommended opportunity: competitor_baseline (high, medium)"));
    assert!(text.contains("priority counts: high=1 medium=1 low=0 other=0"));
}

#[test]
fn opportunities_json_filters_product_opportunities_by_priority() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_opportunities(
        dir.path(),
        &config,
        &registry,
        vec!["--priority".into(), "medium".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.opportunities.v1");
    assert_eq!(value["filter"]["priority"], "medium");
    assert_eq!(value["opportunityCount"], 1);
    assert_eq!(value["totalOpportunityCount"], 2);
    assert_eq!(value["filteredOutOpportunityCount"], 1);
    assert_eq!(value["summary"]["priorityFilter"], "medium");
    assert_eq!(value["summary"]["effortFilter"], Value::Null);
    assert_eq!(value["summary"]["opportunityCount"], 1);
    assert_eq!(value["summary"]["totalOpportunityCount"], 2);
    assert_eq!(value["summary"]["filteredOutOpportunityCount"], 1);
    assert_eq!(value["availablePriorityCounts"]["high"], 1);
    assert_eq!(value["availablePriorityCounts"]["medium"], 1);
    assert_eq!(value["opportunityPriorityCounts"]["medium"], 1);
    assert_eq!(
        value["recommendedOpportunity"]["id"],
        "product_loop_experience"
    );
    assert_eq!(
        value["summary"]["recommendedOpportunityId"],
        "product_loop_experience"
    );
    let opportunities = value["opportunities"].as_array().unwrap();
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0]["id"], "product_loop_experience");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_eq!(
        next_actions,
        vec![
            "deepcli round --json".to_string(),
            "deepcli recipes sota --json".to_string()
        ]
    );
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["checklist"][0]["command"]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("filter: priority=medium"));
}

#[test]
fn opportunities_json_filters_product_opportunities_by_effort() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_opportunities(
        dir.path(),
        &config,
        &registry,
        vec!["--effort".into(), "low".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.opportunities.v1");
    assert_eq!(value["filter"]["priority"], Value::Null);
    assert_eq!(value["filter"]["effort"], "low");
    assert_eq!(value["opportunityCount"], 1);
    assert_eq!(value["totalOpportunityCount"], 2);
    assert_eq!(value["filteredOutOpportunityCount"], 1);
    assert_eq!(value["availableEffortCounts"]["medium"], 1);
    assert_eq!(value["availableEffortCounts"]["low"], 1);
    assert_eq!(value["opportunityEffortCounts"]["low"], 1);
    assert_eq!(
        value["recommendedOpportunity"]["id"],
        "product_loop_experience"
    );
    let opportunities = value["opportunities"].as_array().unwrap();
    assert_eq!(opportunities.len(), 1);
    assert_eq!(opportunities[0]["effort"], "low");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_eq!(
        next_actions,
        vec![
            "deepcli round --json".to_string(),
            "deepcli recipes sota --json".to_string()
        ]
    );
    assert_checklist_matches_executable_actions(&value, &next_actions);

    let text = handle_opportunities(
        dir.path(),
        &config,
        &registry,
        vec!["--effort".into(), "low".into()],
    )
    .unwrap();
    assert!(text.contains("filter: effort=low"));
    assert!(text.contains("effort counts: high=0 medium=0 low=1 other=0"));
}

#[test]
fn sota_baseline_next_actions_prefer_current_capture_when_artifacts_are_ready() {
    let dir = tempdir().unwrap();
    let now = Utc::now();
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
        120,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        now + chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
        250,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990103T000000Z-product-selftest.json",
        now + chrono::Duration::seconds(3),
        "selftest",
        "selftest",
        "passed",
        30,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990104T000000Z-product-scorecard.json",
        now + chrono::Duration::seconds(4),
        "scorecard",
        "scorecard",
        "passed",
        10,
    );

    assert_eq!(
            sota_baseline_next_actions(dir.path()),
            vec![
                "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json",
                "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json",
            ]
        );

    let current = dir.path().join(".deepcli/baselines/current-main.json");
    fs::create_dir_all(current.parent().unwrap()).unwrap();
    fs::write(
        &current,
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "current-main",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": "passed",
                    "durationMs": 120
                },
                {
                    "suite": "product",
                    "case": "preflight-quick",
                    "status": "passed",
                    "durationMs": 250
                },
                {
                    "suite": "product",
                    "case": "selftest",
                    "status": "passed",
                    "durationMs": 30
                },
                {
                    "suite": "product",
                    "case": "scorecard",
                    "status": "passed",
                    "durationMs": 10
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
            sota_baseline_next_actions(dir.path()),
            vec!["deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"]
        );

    let baseline = dir.path().join(".deepcli/baselines/competitor.json");
    fs::create_dir_all(baseline.parent().unwrap()).unwrap();
    fs::write(&baseline, "{}\n").unwrap();

    assert_eq!(
        sota_baseline_next_actions(dir.path()),
        vec!["deepcli benchmark baselines --json"]
    );

    write_ready_competitor_baseline(dir.path());

    assert_eq!(
        sota_baseline_next_actions(dir.path()),
        vec!["deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"]
    );
}

#[test]
fn scorecard_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_scorecard(
        dir.path(),
        &config,
        &registry,
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/scorecard.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.scorecard.v1");
    assert_eq!(value["status"], "needs_attention");
    assert!(value["percent"].as_u64().unwrap() <= 100);
    assert!(value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|category| category["id"] == "benchmark_evidence"));
    assert!(value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|category| category["id"] == "verification_delivery"));
    let categories = value["categories"].as_array().unwrap();
    for category in categories {
        let checklist = category["checklist"].as_array().unwrap();
        assert!(
            !checklist.is_empty(),
            "scorecard category should expose checklist items: {category:?}"
        );
        for (index, item) in checklist.iter().enumerate() {
            assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
            assert!(item["label"].as_str().unwrap().len() >= 3);
            let command = item["command"].as_str().unwrap();
            assert!(
                command.starts_with("deepcli "),
                "scorecard checklist command should be directly executable: {command}"
            );
            assert!(
                !command.contains('<'),
                "scorecard checklist command should not contain placeholders: {command}"
            );
        }
    }
    let command_discovery = categories
        .iter()
        .find(|category| category["id"] == "command_discovery")
        .unwrap();
    assert_eq!(
        command_discovery["checklist"][0]["command"],
        "deepcli quickstart --json"
    );
    assert_eq!(
        command_discovery["checklist"][0]["label"],
        "Open quickstart readiness"
    );
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli preflight --json"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli scorecard"));
    assert!(!dir.path().join(".deepcli/sessions").exists());

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/scorecard.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn scorecard_json_explains_raw_and_normalized_score_scale() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    let now = Utc::now();
    for preset in MEANINGFUL_BENCHMARK_PRESETS {
        write_benchmark_status_test_artifact(
            dir.path(),
            &format!("20990101T000000Z-product-{preset}.json"),
            now,
            preset,
            preset,
            "passed",
        );
    }
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.scorecard.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["normalizedScore"], value["percent"]);
    assert_eq!(value["normalizedScore"].as_u64().unwrap(), 100);
    assert_eq!(value["scoreScale"]["score"], "raw_points");
    assert_eq!(value["scoreScale"]["normalizedScore"], "percent_0_100");
    assert_eq!(value["scoreScale"]["display"], "normalizedScore");
    assert_eq!(value["opportunityEffortCounts"]["medium"], 1);
    assert_eq!(value["opportunityEffortCounts"]["low"], 1);

    let text = handle_scorecard(dir.path(), &config, &registry, Vec::new()).unwrap();
    assert!(text.contains("raw score: "));
    assert!(text.contains("normalized score: 100/100"));

    let report = build_scorecard_report(dir.path(), &config, &registry);
    let summary = scorecard_summary_json(&report);
    assert_eq!(summary["normalizedScore"], summary["percent"]);
    assert_eq!(summary["scoreScale"]["display"], "normalizedScore");
    assert_eq!(summary["opportunityEffortCounts"]["medium"], 1);
    assert_eq!(summary["opportunityEffortCounts"]["low"], 1);

    let round_report = build_round_report(dir.path(), &config, &registry, 85, None);
    let round_text = format_round_text(
        dir.path(),
        RoundTextInput {
            status: round_report.status,
            score_threshold: round_report.score_threshold,
            scorecard: &round_report.scorecard,
            benchmark: &round_report.benchmark,
            benchmark_run: round_report.benchmark_run.as_ref(),
            goal: round_report.goal.as_ref(),
            gates: &round_report.gates,
            gaps: &round_report.gaps,
            next_actions: &round_report.next_actions,
            opportunities: &round_report.opportunities,
        },
    );
    assert!(round_text.contains("scorecard: raw score "));
    assert!(round_text.contains("normalized score 100/100"));
}

#[test]
fn scorecard_next_actions_prioritize_benchmark_gap_remediation() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"].as_array().unwrap();

    assert!(value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .all(|gap| gap.as_str().unwrap().starts_with("benchmark_evidence:")));
    assert_eq!(
        next_actions.first().unwrap().as_str().unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap().starts_with("run `/")));
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli scorecard --json"));
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli round --json"));
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli quickstart --json"));

    let benchmark_category = value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["id"] == "benchmark_evidence")
        .unwrap();
    assert_eq!(
        benchmark_category["nextActions"]
            .as_array()
            .unwrap()
            .first()
            .unwrap()
            .as_str()
            .unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert!(!benchmark_category["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().starts_with("run `/")));
    assert!(!benchmark_category["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli scorecard --json"));
    assert!(!benchmark_category["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli round --json"));

    let command_discovery_category = value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["id"] == "command_discovery")
        .unwrap();
    assert!(command_discovery_category["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli quickstart --json"));
}

#[test]
fn scorecard_ready_next_actions_focus_on_sustaining_product_loop() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    let now = Utc::now();
    for preset in MEANINGFUL_BENCHMARK_PRESETS {
        write_benchmark_status_test_artifact(
            dir.path(),
            &format!("20990101T000000Z-product-{preset}.json"),
            now,
            preset,
            preset,
            "passed",
        );
    }
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action.as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(value["status"], "ok");
    assert!(value["gaps"].as_array().unwrap().is_empty());
    assert_eq!(
            next_actions,
            vec![
                "deepcli round --json --run-benchmark --fail-on-command",
                "deepcli recipes sota --json",
                "deepcli opportunities --json",
                "deepcli benchmark trends --json",
                "deepcli benchmark status --json",
                "deepcli preflight --json",
                "deepcli gate --json",
                "deepcli benchmark baselines --json",
                "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json",
                "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json",
            ]
        );
    let checklist = value["checklist"].as_array().unwrap();
    assert_eq!(checklist.len(), next_actions.len());
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"].as_str().unwrap(), next_actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
    assert_eq!(
        checklist[0]["label"].as_str(),
        Some("Refresh benchmark evidence")
    );
    assert_eq!(
        checklist[7]["label"].as_str(),
        Some("List benchmark baselines")
    );
    assert_eq!(
        checklist[8]["label"].as_str(),
        Some("Capture current benchmark baseline")
    );

    let command_discovery_category = value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["id"] == "command_discovery")
        .unwrap();
    assert!(command_discovery_category["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli quickstart --json"));
}

#[test]
fn scorecard_ready_next_actions_compare_when_default_baseline_exists() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_ready_competitor_baseline(dir.path());
    let now = Utc::now();
    for preset in MEANINGFUL_BENCHMARK_PRESETS {
        write_benchmark_status_test_artifact(
            dir.path(),
            &format!("20990101T000000Z-product-{preset}.json"),
            now,
            preset,
            preset,
            "passed",
        );
    }
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    let baselines_index = next_actions
        .iter()
        .position(|action| action == "deepcli benchmark baselines --json")
        .expect("scorecard should expose baseline inventory before compare");
    let compare_index = next_actions
        .iter()
        .position(|action| {
            action
                == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
        })
        .expect("scorecard should still expose baseline compare");
    assert!(baselines_index < compare_index);
    assert!(!next_actions.iter().any(|action| {
            action == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
    assert_checklist_matches_executable_actions(&value, &next_actions);
}

#[test]
fn product_loop_reports_surface_sota_recipe_next_action() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let scorecard =
        handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
    assert!(scorecard_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli recipes sota --json"));

    let round = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let round_value: Value = serde_json::from_str(&round).unwrap();
    assert!(round_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli recipes sota --json"));

    let benchmark_status = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let benchmark_status_value: Value = serde_json::from_str(&benchmark_status).unwrap();
    assert!(benchmark_status_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli recipes sota --json"));
}

#[test]
fn scorecard_fail_below_and_output_safety_are_enforced() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let failure = handle_scorecard(
        dir.path(),
        &config,
        &registry,
        vec!["--json".into(), "--fail-below".into(), "100".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(failure.contains("deepcli.scorecard.v1"));

    let bad_threshold = handle_scorecard(
        dir.path(),
        &config,
        &registry,
        vec!["--fail-below".into(), "101".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(bad_threshold.contains("between 0 and 100"));

    let traversal = handle_scorecard(
        dir.path(),
        &config,
        &registry,
        vec!["--output".into(), "../scorecard.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../scorecard.json").exists());
}

#[test]
fn round_json_output_aggregates_scorecard_and_benchmark_status() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(
        dir.path(),
        &config,
        &registry,
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/round.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.round.v1");
    assert_eq!(value["status"], "needs_attention");
    assert_eq!(value["ready"], false);
    assert_eq!(value["scoreThreshold"], 90);
    assert_eq!(value["summary"]["status"], value["status"]);
    assert_eq!(value["summary"]["ready"], value["ready"]);
    assert_eq!(value["summary"]["scoreThreshold"], value["scoreThreshold"]);
    assert_eq!(
        value["summary"]["scorecardPercent"],
        value["scorecard"]["percent"]
    );
    assert_eq!(
        value["summary"]["benchmarkStatus"],
        value["benchmarkStatus"]["status"]
    );
    assert_eq!(
        value["summary"]["gateCount"],
        value["gates"].as_array().unwrap().len()
    );
    assert_eq!(
        value["summary"]["gapCount"],
        value["gaps"].as_array().unwrap().len()
    );
    assert_eq!(value["summary"]["opportunityCount"], 0);
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["checklist"][0]["command"]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    assert_eq!(value["scorecard"]["schema"], "deepcli.scorecard.summary.v1");
    assert_eq!(
        value["benchmarkStatus"]["schema"],
        "deepcli.benchmark.status.v1"
    );
    assert_eq!(value["benchmarkStatus"]["status"], "missing");
    assert!(value["benchmarkRun"].is_null());
    assert!(value["gates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gate| gate["id"] == "benchmark_evidence" && gate["status"] == "failed"));
    for gate in value["gates"].as_array().unwrap() {
        let checklist = gate["checklist"].as_array().unwrap();
        if gate["nextAction"].is_null() {
            assert!(
                checklist.is_empty(),
                "round gate without nextAction should expose an empty checklist: {gate:?}"
            );
        } else {
            assert_eq!(checklist.len(), 1);
            assert_eq!(checklist[0]["step"], 1);
            assert_eq!(checklist[0]["command"], gate["nextAction"]);
            assert!(checklist[0]["command"]
                .as_str()
                .unwrap()
                .starts_with("deepcli "));
            assert!(
                !checklist[0]["command"].as_str().unwrap().contains('<'),
                "round gate checklist command should not contain placeholders: {gate:?}"
            );
            assert!(checklist[0]["label"].as_str().unwrap().len() >= 3);
        }
    }
    let benchmark_gate = value["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["id"] == "benchmark_evidence")
        .unwrap();
    assert_eq!(
        benchmark_gate["checklist"][0]["command"].as_str(),
        Some("deepcli round --json --run-benchmark --fail-on-command")
    );
    assert_eq!(
        benchmark_gate["checklist"][0]["label"].as_str(),
        Some("Refresh benchmark evidence")
    );
    assert!(value["gates"].as_array().unwrap().iter().any(|gate| {
        gate["id"] == "benchmark_evidence"
            && gate["summary"]
                .as_str()
                .unwrap()
                .contains("missing presets: cargo-test")
    }));
    let benchmark_category = value["scorecard"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["id"] == "benchmark_evidence")
        .unwrap();
    assert_eq!(
        benchmark_category["nextActions"][0].as_str(),
        Some("deepcli round --json --run-benchmark --fail-on-command")
    );
    assert_eq!(
        benchmark_category["checklist"][0]["command"].as_str(),
        Some("deepcli round --json --run-benchmark --fail-on-command")
    );
    assert_eq!(
        benchmark_category["checklist"][0]["label"].as_str(),
        Some("Refresh benchmark evidence")
    );
    let cargo_test_benchmark = benchmark_category["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["command"].as_str()
                == Some("deepcli benchmark run --preset cargo-test --json --fail-on-command")
        })
        .unwrap();
    assert_eq!(
        cargo_test_benchmark["label"].as_str(),
        Some("Run cargo-test benchmark")
    );
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains("deepcli benchmark run")));
    let next_actions = value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action.as_str().unwrap())
        .collect::<Vec<_>>();
    let checklist = value["checklist"].as_array().unwrap();
    assert_eq!(checklist.len(), next_actions.len());
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"].as_str().unwrap(), next_actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
    let benchmark_refresh = checklist
        .iter()
        .find(|item| {
            item["command"].as_str()
                == Some("deepcli round --json --run-benchmark --fail-on-command")
        })
        .unwrap();
    assert_eq!(
        benchmark_refresh["label"].as_str(),
        Some("Refresh benchmark evidence")
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli product round"));
    assert!(!dir.path().join(".deepcli/sessions").exists());

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/round.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn round_scorecard_gate_tracks_threshold_separately_from_gaps() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(
        dir.path(),
        &config,
        &registry,
        vec!["--json".into(), "--fail-below".into(), "0".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let gates = value["gates"].as_array().unwrap();

    assert!(value["ready"].as_bool().is_some_and(|ready| !ready));
    assert!(value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap.as_str().unwrap().starts_with("benchmark_evidence:")));
    assert!(gates.iter().any(|gate| {
        gate["id"] == "scorecard"
            && gate["status"] == "passed"
            && gate["summary"]
                .as_str()
                .unwrap()
                .contains("meets the 0% round threshold")
    }));
    assert!(gates
        .iter()
        .any(|gate| gate["id"] == "benchmark_evidence" && gate["status"] == "failed"));
}

#[test]
fn round_next_actions_prioritize_failing_benchmark_gate_when_scorecard_threshold_passes() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"].as_array().unwrap();

    assert!(value["gates"].as_array().unwrap().iter().any(|gate| {
        gate["id"] == "scorecard" && gate["status"] == "passed" && gate["nextAction"].is_null()
    }));
    assert!(value["gates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gate| gate["id"] == "benchmark_evidence" && gate["status"] == "failed"));
    assert!(value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .all(|gap| gap.as_str().unwrap().starts_with("benchmark_evidence:")));
    assert_eq!(
        next_actions.first().unwrap().as_str().unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli scorecard --json"));
    assert!(!next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli round --json"));
}

#[test]
fn round_ready_next_actions_include_baseline_template_when_default_baseline_is_missing() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action.as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(value["status"], "ready");
    assert_eq!(value["ready"], true);
    assert!(value["gaps"].as_array().unwrap().is_empty());
    assert_eq!(
            next_actions,
            vec![
                "deepcli preflight --json",
                "deepcli gate --json",
                "deepcli recipes sota --json",
                "deepcli opportunities --json",
                "deepcli benchmark baselines --json",
                "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json",
                "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json",
            ]
        );
    let top_next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &top_next_actions);
    assert_eq!(
        value["checklist"][2]["label"],
        "Open SOTA product loop recipe"
    );
    assert_eq!(value["checklist"][3]["label"], "Open product opportunities");
    assert_eq!(value["checklist"][4]["label"], "List benchmark baselines");
    let benchmark_gate = value["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["id"] == "benchmark_evidence")
        .unwrap();
    assert_eq!(
        benchmark_gate["checklist"][0]["command"].as_str(),
        Some("deepcli benchmark summary --json")
    );
    assert_eq!(
        benchmark_gate["checklist"][0]["label"].as_str(),
        Some("Review benchmark summary")
    );
    assert!(value["report"].as_str().unwrap().contains(
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        ));
    assert!(value["report"].as_str().unwrap().contains(
        "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
    ));
}

#[test]
fn round_ready_routes_unfilled_default_baseline_to_inventory() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let baselines_dir = dir.path().join(".deepcli/baselines");
    fs::create_dir_all(&baselines_dir).unwrap();
    fs::write(
        baselines_dir.join("competitor.json"),
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "competitor",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": null,
                    "durationMs": null
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["status"], "ready");
    assert_eq!(
        next_actions,
        vec![
            "deepcli preflight --json",
            "deepcli gate --json",
            "deepcli recipes sota --json",
            "deepcli opportunities --json",
            "deepcli benchmark baselines --json",
        ]
    );
    assert!(!next_actions.iter().any(|action| {
        action == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
    }));
    let baseline_opportunity = value["opportunities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|opportunity| opportunity["id"] == "competitor_baseline")
        .unwrap();
    assert_eq!(baseline_opportunity["title"], "Prepare Competitor Baseline");
    assert_eq!(
        baseline_opportunity["nextActions"][0],
        "deepcli benchmark baselines --json"
    );
    assert_eq!(
        baseline_opportunity["checklist"][0]["label"],
        "List benchmark baselines"
    );
}

#[test]
fn round_ready_surfaces_non_blocking_product_opportunities() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["status"], "ready");
    assert!(value["gaps"].as_array().unwrap().is_empty());
    let opportunities = value["opportunities"].as_array().unwrap();
    assert_eq!(value["summary"]["status"], "ready");
    assert_eq!(value["summary"]["ready"], true);
    assert_eq!(value["summary"]["gapCount"], 0);
    assert_eq!(value["summary"]["opportunityCount"], opportunities.len());
    assert_eq!(
        value["summary"]["recommendedOpportunityId"],
        value["recommendedOpportunity"]["id"]
    );
    assert_eq!(
        value["summary"]["benchmarkFreshnessStatus"],
        value["benchmarkStatus"]["summary"]["freshnessStatus"]
    );
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["checklist"][0]["command"]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    assert!(
        !opportunities.is_empty(),
        "ready round should still expose next product opportunities"
    );
    assert_eq!(
        value["recommendedOpportunity"]["id"],
        opportunities[0]["id"]
    );
    assert_eq!(value["opportunityPriorityCounts"]["high"], 1);
    assert_eq!(value["opportunityPriorityCounts"]["medium"], 1);
    assert_eq!(value["opportunityEffortCounts"]["medium"], 1);
    assert_eq!(value["opportunityEffortCounts"]["low"], 1);
    assert_eq!(
        value["scorecard"]["recommendedOpportunity"]["id"],
        opportunities[0]["id"]
    );
    assert_eq!(value["scorecard"]["opportunityPriorityCounts"]["high"], 1);
    assert_eq!(value["scorecard"]["opportunityEffortCounts"]["medium"], 1);
    assert_eq!(value["scorecard"]["opportunityEffortCounts"]["low"], 1);
    let baseline_opportunity = opportunities
        .iter()
        .find(|opportunity| opportunity["id"] == "competitor_baseline")
        .expect("ready round should recommend competitor baseline setup");
    assert_eq!(baseline_opportunity["status"], "available");
    assert_eq!(baseline_opportunity["effort"], "medium");
    assert!(baseline_opportunity["summary"]
        .as_str()
        .unwrap()
        .contains("baseline"));
    assert_eq!(
        baseline_opportunity["nextActions"][0],
        "deepcli benchmark baselines --json"
    );
    assert!(json_string_array(&baseline_opportunity["nextActions"])
            .iter()
            .any(|action| action
                == "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"));
    assert_eq!(
        baseline_opportunity["checklist"][0]["command"],
        baseline_opportunity["nextActions"][0]
    );
    assert_eq!(
        baseline_opportunity["checklist"][0]["label"],
        "List benchmark baselines"
    );
    assert_eq!(
        value["scorecard"]["opportunities"][0]["id"],
        baseline_opportunity["id"]
    );
    let loop_opportunity = opportunities
        .iter()
        .find(|opportunity| opportunity["id"] == "product_loop_experience")
        .expect("ready round should recommend exercising the product loop");
    assert_eq!(loop_opportunity["effort"], "low");
    assert_eq!(loop_opportunity["priority"], "medium");
    assert!(value["report"].as_str().unwrap().contains("opportunities:"));

    let text = handle_round(dir.path(), &config, &registry, Vec::new()).unwrap();
    assert!(text.contains("recommended opportunity: competitor_baseline (high, medium)"));
    assert!(text.contains("priority counts: high=1 medium=1 low=0 other=0"));
}

#[test]
fn round_ready_product_opportunity_keeps_compare_after_baseline_inventory() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    write_ready_competitor_baseline(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let baseline_opportunity = value["opportunities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|opportunity| opportunity["id"] == "competitor_baseline")
        .expect("ready round should recommend competitor baseline comparison");
    let next_actions = json_string_array(&baseline_opportunity["nextActions"]);

    assert_eq!(next_actions[0], "deepcli benchmark baselines --json");
    assert!(next_actions.contains(
        &"deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
            .to_string()
    ));
    assert!(!next_actions.contains(
            &"deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
                .to_string()
        ));
}

#[test]
fn round_ready_next_actions_compare_when_default_baseline_exists() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    write_round_ready_benchmark_history(dir.path());
    write_ready_competitor_baseline(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"].as_array().unwrap();

    assert_eq!(value["status"], "ready");
    assert!(next_actions.iter().any(|action| {
        action.as_str().unwrap()
            == "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
    }));
    assert!(!next_actions.iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
}

#[test]
fn round_surfaces_insufficient_benchmark_trend_history() {
    let dir = tempdir().unwrap();
    write_round_scorecard_ready_fixture(dir.path());
    let now = Utc::now();
    for preset in MEANINGFUL_BENCHMARK_PRESETS {
        write_benchmark_status_test_artifact(
            dir.path(),
            &format!("20990101T000000Z-product-{preset}.json"),
            now,
            preset,
            preset,
            "passed",
        );
    }
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"].as_array().unwrap();

    assert_eq!(value["status"], "needs_attention");
    assert_eq!(value["ready"], false);
    assert!(value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap.as_str().unwrap().starts_with("benchmark_trends:")));
    assert!(value["gates"].as_array().unwrap().iter().any(|gate| {
        gate["id"] == "benchmark_trends"
            && gate["status"] == "failed"
            && gate["summary"]
                .as_str()
                .unwrap()
                .contains("insufficient_history")
            && gate["nextAction"] == "deepcli round --json --run-benchmark --fail-on-command"
    }));
    assert_eq!(
        next_actions.first().unwrap().as_str().unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("benchmark_trends"));
}

#[test]
fn round_can_run_benchmark_suite_before_reporting() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_round(
        dir.path(),
        &config,
        &registry,
        vec![
            "--json".into(),
            "--run-benchmark".into(),
            "--preset".into(),
            "smoke".into(),
            "--output".into(),
            ".deepcli/exports/round-run.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.round.v1");
    assert_eq!(
        value["benchmarkRun"]["schema"],
        "deepcli.benchmark.suite.v1"
    );
    assert_eq!(value["benchmarkRun"]["status"], "passed");
    assert_eq!(value["benchmarkRun"]["presetCount"], 1);
    assert_eq!(value["benchmarkRun"]["requestedPresets"][0], "smoke");
    assert_eq!(value["benchmarkRun"]["artifacts"][0]["preset"], "smoke");
    assert_eq!(value["benchmarkStatus"]["artifactCount"], 1);
    assert_eq!(value["benchmarkStatus"]["status"], "weak");
    assert!(dir.path().join(".deepcli/benchmarks").exists());
    assert!(dir.path().join(".deepcli/exports/round-run.json").exists());
    assert!(!dir.path().join(".deepcli/sessions").exists());
}

#[test]
fn round_surfaces_latest_goal_readiness_when_goal_exists() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();

    handle_goal(
        dir.path(),
        Some(session.id().to_string()),
        vec![
            "实现全部需求".to_string(),
            "--acceptance-cmd".to_string(),
            "cargo test".to_string(),
        ],
    )
    .unwrap();

    let output = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.round.v1");
    assert_eq!(
        value["goalStatus"]["schema"],
        "deepcli.goal.status.summary.v1"
    );
    assert_eq!(value["goalStatus"]["ready"], false);
    assert_eq!(value["goalStatus"]["sessionSource"], "latest_with_goal");
    assert_eq!(
        value["goalStatus"]["session"]["id"],
        session.id().to_string()
    );
    assert!(value["gates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gate| gate["id"] == "goal_readiness" && gate["status"] == "failed"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action
            .as_str()
            .unwrap()
            .contains("deepcli goal gate --json")));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("goal: ready=false"));
}

#[test]
fn round_strict_mode_and_output_safety_are_enforced() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let failure = handle_round(
        dir.path(),
        &config,
        &registry,
        vec!["--json".into(), "--fail-on-gaps".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(failure.contains("deepcli.round.v1"));

    let bad_threshold = handle_round(
        dir.path(),
        &config,
        &registry,
        vec!["--fail-below".into(), "101".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(bad_threshold.contains("between 0 and 100"));

    let traversal = handle_round(
        dir.path(),
        &config,
        &registry,
        vec!["--output".into(), "../round.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../round.json").exists());
}

#[test]
fn benchmark_run_executes_command_and_records_artifact() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--json".into(),
            "--suite".into(),
            "local".into(),
            "--case".into(),
            "echo".into(),
            "--command".into(),
            "printf bench".into(),
            "--timeout".into(),
            "5".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], BENCHMARK_ARTIFACT_SCHEMA);
    assert_eq!(value["execution"]["mode"], "command");
    assert_eq!(value["execution"]["ranByDeepcli"], true);
    assert_eq!(value["execution"]["status"], "passed");
    assert_eq!(value["execution"]["commands"][0]["exitCode"], 0);
    assert_eq!(value["execution"]["commands"][0]["stdoutSample"], "bench");
    assert_eq!(value["scorecard"]["schema"], "deepcli.scorecard.summary.v1");
    assert!(value["scorecard"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|category| category["id"] == "benchmark_evidence" && category["status"] != "strong"));
    let artifact_path = value["artifactPath"].as_str().unwrap();
    assert!(dir.path().join(artifact_path).exists());
}

#[test]
fn benchmark_run_fail_on_command_writes_artifact_before_exit() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let failure = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--json".into(),
            "--command".into(),
            "exit 7".into(),
            "--fail-on-command".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    let value: Value = serde_json::from_str(&failure).unwrap();
    assert_eq!(value["execution"]["status"], "failed");
    assert_eq!(value["execution"]["commands"][0]["exitCode"], 7);
    assert!(dir
        .path()
        .join(value["artifactPath"].as_str().unwrap())
        .exists());

    let missing = handle_benchmark(dir.path(), &config, &registry, vec!["run".into()])
        .unwrap_err()
        .to_string();
    assert!(missing.contains("requires `--preset <name>`"));
}

#[test]
fn benchmark_presets_are_listed_and_can_run_smoke_evidence() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let presets = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["presets".into(), "--json".into()],
    )
    .unwrap();
    let presets_value: Value = serde_json::from_str(&presets).unwrap();
    assert_eq!(presets_value["schema"], "deepcli.benchmark.presets.v1");
    assert_eq!(presets_value["summary"]["status"], "ok");
    assert_eq!(presets_value["summary"]["presetCount"], 5);
    assert_eq!(presets_value["summary"]["defaultSuitePresetCount"], 4);
    assert_eq!(presets_value["summary"]["requiredEvidencePresetCount"], 4);
    assert_eq!(presets_value["summary"]["optionalPresetCount"], 1);
    assert_eq!(
        presets_value["summary"]["defaultSuiteAction"],
        "deepcli benchmark run-suite --json --fail-on-command"
    );
    assert_eq!(
        presets_value["summary"]["recommendedAction"],
        presets_value["checklist"][0]["command"]
    );
    assert_eq!(
        presets_value["summary"]["recommendedActionLabel"],
        presets_value["checklist"][0]["label"]
    );
    assert_eq!(
        presets_value["summary"]["defaultSuitePresets"],
        json!(["cargo-test", "preflight-quick", "selftest", "scorecard"])
    );
    assert_eq!(
        presets_value["summary"]["requiredEvidencePresets"],
        json!(["cargo-test", "preflight-quick", "selftest", "scorecard"])
    );
    assert!(presets_value["presets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|preset| preset["name"] == "cargo-test"));
    let cargo_preset = presets_value["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|preset| preset["name"] == "cargo-test")
        .unwrap();
    assert_eq!(cargo_preset["defaultSuite"], true);
    assert_eq!(cargo_preset["requiredEvidence"], true);
    let smoke_preset = presets_value["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|preset| preset["name"] == "smoke")
        .unwrap();
    assert_eq!(smoke_preset["defaultSuite"], false);
    assert_eq!(smoke_preset["requiredEvidence"], false);
    assert!(presets_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains("run --preset cargo-test")));
    assert_benchmark_checklist_matches_next_actions(&presets_value);

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--json".into(),
            "--preset".into(),
            "smoke".into(),
            "--fail-on-command".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], BENCHMARK_ARTIFACT_SCHEMA);
    assert_eq!(value["preset"], "smoke");
    assert_eq!(value["suite"], "product");
    assert_eq!(value["case"], "smoke");
    assert_eq!(value["execution"]["status"], "passed");
    assert_eq!(
        value["execution"]["commands"][0]["stdoutSample"],
        "deepcli-benchmark-smoke"
    );
    assert_eq!(
        value["declaredCommands"][0],
        "printf deepcli-benchmark-smoke"
    );
    assert_benchmark_checklist_matches_next_actions(&value);

    let conflict = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--preset".into(),
            "smoke".into(),
            "--command".into(),
            "printf nope".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(conflict.contains("cannot be combined"));

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "presets".into(),
            "--output".into(),
            "../presets.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../presets.json").exists());
}

#[test]
fn benchmark_run_suite_executes_selected_presets_and_reports_status() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run-suite".into(),
            "--json".into(),
            "--preset".into(),
            "smoke".into(),
            "--output".into(),
            ".deepcli/exports/benchmark-suite.json".into(),
            "--fail-on-command".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], BENCHMARK_SUITE_SCHEMA);
    assert_eq!(value["status"], "passed");
    assert_eq!(value["presetCount"], 1);
    assert_eq!(value["passedCount"], 1);
    assert_eq!(value["failedCount"], 0);
    assert_eq!(value["timeoutCount"], 0);
    assert_eq!(value["requestedPresets"][0], "smoke");
    assert_eq!(value["artifacts"][0]["preset"], "smoke");
    assert_eq!(value["artifacts"][0]["status"], "passed");
    assert!(value["artifacts"][0]["artifactPath"]
        .as_str()
        .unwrap()
        .starts_with(".deepcli/benchmarks/"));
    assert_eq!(value["benchmarkStatus"]["schema"], BENCHMARK_STATUS_SCHEMA);
    assert_eq!(value["benchmarkStatus"]["status"], "weak");
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains("benchmark trends")));
    assert_benchmark_checklist_matches_next_actions(&value);
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli benchmark run-suite"));
    assert!(dir
        .path()
        .join(value["artifacts"][0]["artifactPath"].as_str().unwrap())
        .exists());
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/benchmark-suite.json")).unwrap();
    assert_eq!(written, output);

    let text = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["suite".into(), "--preset".into(), "smoke".into()],
    )
    .unwrap();
    assert!(text.contains("deepcli benchmark run-suite"));
    assert!(text.contains("smoke: status=passed"));

    let duplicate_presets = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run-suite".into(),
            "--json".into(),
            "--presets".into(),
            "smoke,smoke".into(),
        ],
    )
    .unwrap();
    let duplicate_value: Value = serde_json::from_str(&duplicate_presets).unwrap();
    assert_eq!(duplicate_value["presetCount"], 1);

    let unknown = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["run-suite".into(), "--preset".into(), "unknown".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(unknown.contains("unknown benchmark preset `unknown`"));

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run-suite".into(),
            "--output".into(),
            "../suite.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../suite.json").exists());
}

fn write_benchmark_status_test_artifact(
    workspace: &std::path::Path,
    file_name: &str,
    created_at: DateTime<Utc>,
    preset: &str,
    case_name: &str,
    status: &str,
) -> String {
    write_benchmark_status_test_artifact_with_duration(
        workspace, file_name, created_at, preset, case_name, status, 42,
    )
}

fn write_benchmark_status_test_artifact_with_duration(
    workspace: &std::path::Path,
    file_name: &str,
    created_at: DateTime<Utc>,
    preset: &str,
    case_name: &str,
    status: &str,
    duration_ms: u64,
) -> String {
    let relative_path = format!(".deepcli/benchmarks/{file_name}");
    let artifact = json!({
        "schema": BENCHMARK_ARTIFACT_SCHEMA,
        "createdAt": created_at.to_rfc3339(),
        "artifactPath": relative_path,
        "suite": "product",
        "case": case_name,
        "preset": preset,
        "declaredCommands": ["cargo test"],
        "execution": {
            "mode": "command",
            "ranByDeepcli": true,
            "status": status,
            "commands": [{
                "command": "cargo test",
                "status": status,
                "exitCode": if status == "passed" { Some(0) } else { Some(1) },
                "timedOut": status == "timeout",
                "durationMs": duration_ms,
                "stdoutChars": 2,
                "stderrChars": 0,
                "stdoutSample": "ok",
                "stderrSample": "",
                "error": Value::Null,
            }],
        },
    });
    let path = workspace.join(&relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_string_pretty(&artifact).unwrap()).unwrap();
    relative_path
}

#[test]
fn benchmark_status_classifies_missing_smoke_and_ready_evidence() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let missing = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let missing_value: Value = serde_json::from_str(&missing).unwrap();
    assert_eq!(missing_value["schema"], BENCHMARK_STATUS_SCHEMA);
    assert_eq!(missing_value["status"], "missing");
    assert_eq!(missing_value["ready"], false);
    assert_eq!(missing_value["hasGaps"], true);
    assert_eq!(missing_value["artifactCount"], 0);
    assert!(missing_value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli benchmark status"));
    assert!(missing_value["report"]
        .as_str()
        .unwrap()
        .contains("status: missing"));
    assert_eq!(missing_value["meaningful"]["passedCount"], 0);
    assert!(missing_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains("run --preset cargo-test")));
    assert!(!missing_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli benchmark clean --dry-run --json"));

    let missing_gate = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["gate".into(), "--json".into()],
    )
    .unwrap_err();
    let exit = missing_gate.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);
    let gate_value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(gate_value["schema"], BENCHMARK_STATUS_SCHEMA);
    assert_eq!(gate_value["status"], "missing");

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--output".into(), "../status.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));

    handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--json".into(),
            "--preset".into(),
            "smoke".into(),
            "--fail-on-command".into(),
        ],
    )
    .unwrap();
    let weak = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let weak_value: Value = serde_json::from_str(&weak).unwrap();
    assert_eq!(weak_value["status"], "weak");
    assert_eq!(weak_value["ready"], false);
    assert_eq!(weak_value["totals"]["smokeCount"], 1);
    assert_eq!(weak_value["meaningful"]["executableCount"], 0);
    assert!(weak_value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap.as_str().unwrap().contains("only smoke")));
    assert!(weak_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli benchmark clean --dry-run --json"));

    let scorecard =
        handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
    let benchmark_category = scorecard_value["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["id"] == "benchmark_evidence")
        .unwrap();
    assert_ne!(benchmark_category["status"], "strong");
    assert!(benchmark_category["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap.as_str().unwrap().contains("only smoke")));

    let cargo_path = write_benchmark_status_test_artifact(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        Utc::now() + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
    );
    let incomplete = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let incomplete_value: Value = serde_json::from_str(&incomplete).unwrap();
    assert_eq!(incomplete_value["status"], "incomplete");
    assert_eq!(incomplete_value["ready"], false);
    assert_eq!(incomplete_value["hasGaps"], true);
    assert_eq!(incomplete_value["meaningful"]["passedCount"], 1);
    assert_eq!(
        incomplete_value["latestMeaningfulArtifact"]["artifactPath"],
        cargo_path
    );
    assert!(incomplete_value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap
            .as_str()
            .unwrap()
            .contains("missing required benchmark preset `preflight-quick`")));
    let required_status = incomplete_value["presetCoverage"]["requiredStatus"]
        .as_array()
        .unwrap();
    assert!(required_status.iter().any(|preset| {
        preset["preset"] == "selftest"
            && preset["status"] == "missing"
            && preset["gap"]
                .as_str()
                .unwrap()
                .contains("deepcli benchmark run-suite --json --fail-on-command")
    }));
    assert!(!required_status.iter().any(|preset| preset["gap"]
        .as_str()
        .is_some_and(|gap| gap.contains("run `/benchmark"))));
    assert!(incomplete_value["presetCoverage"]["requiredStatus"]
        .as_array()
        .unwrap()
        .iter()
        .any(|preset| preset["preset"] == "cargo-test" && preset["status"] == "passed"));
    assert!(incomplete_value["presetCoverage"]["requiredStatus"]
        .as_array()
        .unwrap()
        .iter()
        .any(|preset| preset["preset"] == "selftest" && preset["status"] == "missing"));

    write_benchmark_status_test_artifact(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        Utc::now() + chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990103T000000Z-product-selftest.json",
        Utc::now() + chrono::Duration::seconds(3),
        "selftest",
        "selftest",
        "passed",
    );
    let scorecard_path = write_benchmark_status_test_artifact(
        dir.path(),
        "20990104T000000Z-product-scorecard.json",
        Utc::now() + chrono::Duration::seconds(4),
        "scorecard",
        "scorecard",
        "passed",
    );
    let ready = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let ready_value: Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready_value["status"], "ready");
    assert_eq!(ready_value["ready"], true);
    assert_eq!(ready_value["hasGaps"], false);
    assert_eq!(ready_value["meaningful"]["passedCount"], 4);
    assert_eq!(
        ready_value["latestMeaningfulArtifact"]["artifactPath"],
        scorecard_path
    );
    let ready_next_actions = json_string_array(&ready_value["nextActions"]);
    let recipes_index = ready_next_actions
        .iter()
        .position(|action| action == "deepcli recipes sota --json")
        .expect("ready benchmark status should link back to SOTA recipes");
    let baselines_index = ready_next_actions
        .iter()
        .position(|action| action == "deepcli benchmark baselines --json")
        .expect("ready benchmark status should link to baseline inventory");
    let presets_index = ready_next_actions
        .iter()
        .position(|action| action == "deepcli benchmark presets --json")
        .expect("ready benchmark status should keep preset discovery");
    assert!(recipes_index < baselines_index);
    assert!(baselines_index < presets_index);
    assert_checklist_matches_executable_actions(&ready_value, &ready_next_actions);
    assert!(json_checklist_labels(&ready_value).contains(&"List benchmark baselines".to_string()));

    let ready_gate = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["gate".into(), "--json".into()],
    )
    .unwrap();
    let ready_gate_value: Value = serde_json::from_str(&ready_gate).unwrap();
    assert_eq!(ready_gate_value["status"], "ready");
}

#[test]
fn benchmark_status_flags_failing_and_stale_meaningful_evidence() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    write_benchmark_status_test_artifact(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        Utc::now() + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "failed",
    );
    let failing = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let failing_value: Value = serde_json::from_str(&failing).unwrap();
    assert_eq!(failing_value["status"], "failing");
    assert_eq!(failing_value["meaningful"]["failedCount"], 1);

    let stale_dir = tempdir().unwrap();
    let stale_created_at =
        Utc::now() - chrono::Duration::days(BENCHMARK_EVIDENCE_STALE_AFTER_DAYS + 1);
    write_benchmark_status_test_artifact(
        stale_dir.path(),
        "20000101T000000Z-product-cargo-test.json",
        stale_created_at,
        "cargo-test",
        "cargo-test",
        "passed",
    );
    write_benchmark_status_test_artifact(
        stale_dir.path(),
        "20000102T000000Z-product-preflight-quick.json",
        stale_created_at + chrono::Duration::seconds(1),
        "preflight-quick",
        "preflight-quick",
        "passed",
    );
    write_benchmark_status_test_artifact(
        stale_dir.path(),
        "20000103T000000Z-product-selftest.json",
        stale_created_at + chrono::Duration::seconds(2),
        "selftest",
        "selftest",
        "passed",
    );
    write_benchmark_status_test_artifact(
        stale_dir.path(),
        "20000104T000000Z-product-scorecard.json",
        stale_created_at + chrono::Duration::seconds(3),
        "scorecard",
        "scorecard",
        "passed",
    );
    let stale = handle_benchmark(
        stale_dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let stale_value: Value = serde_json::from_str(&stale).unwrap();
    assert_eq!(stale_value["status"], "stale");
    assert!(stale_value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap.as_str().unwrap().contains("older than")));
}

#[test]
fn benchmark_status_surfaces_aging_ready_evidence_freshness() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let latest_created_at = Utc::now() - chrono::Duration::days(2);
    let previous_created_at = latest_created_at - chrono::Duration::hours(1);
    write_round_scorecard_ready_fixture(dir.path());

    write_benchmark_status_test_artifact(
        dir.path(),
        "20981201T000000Z-product-cargo-test.json",
        previous_created_at - chrono::Duration::seconds(3),
        "cargo-test",
        "cargo-test",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20981202T000000Z-product-preflight-quick.json",
        previous_created_at - chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20981203T000000Z-product-selftest.json",
        previous_created_at - chrono::Duration::seconds(1),
        "selftest",
        "selftest",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20981204T000000Z-product-scorecard.json",
        previous_created_at,
        "scorecard",
        "scorecard",
        "passed",
    );

    write_benchmark_status_test_artifact(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        latest_created_at - chrono::Duration::seconds(3),
        "cargo-test",
        "cargo-test",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        latest_created_at - chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990103T000000Z-product-selftest.json",
        latest_created_at - chrono::Duration::seconds(1),
        "selftest",
        "selftest",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990104T000000Z-product-scorecard.json",
        latest_created_at,
        "scorecard",
        "scorecard",
        "passed",
    );

    let status = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&status).unwrap();

    assert_eq!(value["status"], "ready");
    assert_eq!(value["ready"], true);
    assert_eq!(value["freshness"]["status"], "aging");
    assert_eq!(value["freshness"]["latestMeaningfulAge"], "2d");
    assert_eq!(value["freshness"]["refreshRecommended"], true);
    assert_eq!(
        value["freshness"]["refreshAction"],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert_eq!(value["summary"]["status"], "ready");
    assert_eq!(value["summary"]["ready"], true);
    assert_eq!(value["summary"]["artifactCount"], 8);
    assert_eq!(value["summary"]["meaningfulArtifactCount"], 8);
    assert_eq!(value["summary"]["freshnessStatus"], "aging");
    assert_eq!(value["summary"]["refreshRecommended"], true);
    assert_eq!(value["summary"]["requiredPresetCount"], 4);
    assert_eq!(value["summary"]["requiredReadyCount"], 4);
    assert_eq!(
        value["summary"]["recommendedAction"],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    assert_eq!(
        value["nextActions"][0],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("freshness: aging age=2d"));

    let round = handle_round(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let round_value: Value = serde_json::from_str(&round).unwrap();
    assert_eq!(
        round_value["benchmarkStatus"]["freshness"]["status"],
        "aging"
    );
    assert_eq!(
        round_value["nextActions"][0],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert!(round_value["gates"].as_array().unwrap().iter().any(|gate| {
        gate["id"] == "benchmark_evidence"
            && gate["summary"]
                .as_str()
                .unwrap()
                .contains("freshness=aging age=2d")
    }));
    let benchmark_gate = round_value["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["id"] == "benchmark_evidence")
        .unwrap();
    assert_eq!(
        benchmark_gate["nextAction"],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert_eq!(
        benchmark_gate["checklist"][0]["label"],
        "Refresh benchmark evidence"
    );
    assert!(round_value["report"]
        .as_str()
        .unwrap()
        .contains("benchmark: status=ready ready=true"));
    assert!(round_value["report"]
        .as_str()
        .unwrap()
        .contains("freshness=aging age=2d"));
    let round_freshness_opportunity = round_value["opportunities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|opportunity| opportunity["id"] == "benchmark_freshness")
        .expect("aging ready benchmark evidence should be explained as an opportunity");
    assert_eq!(
        round_freshness_opportunity["title"],
        "Refresh Benchmark Evidence"
    );
    assert_eq!(round_freshness_opportunity["effort"], "low");
    assert_eq!(round_freshness_opportunity["priority"], "high");
    assert_eq!(
        round_freshness_opportunity["nextActions"][0],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert_eq!(
        round_freshness_opportunity["checklist"][0]["label"],
        "Refresh benchmark evidence"
    );

    let scorecard =
        handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
    assert_eq!(
        scorecard_value["nextActions"][0],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert!(scorecard_value["opportunities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|opportunity| opportunity["id"] == "benchmark_freshness"));

    let recipes = handle_recipes(
        dir.path(),
        &config,
        &registry,
        vec!["sota".into(), "--json".into()],
    )
    .unwrap();
    let recipes_value: Value = serde_json::from_str(&recipes).unwrap();
    assert_eq!(
        recipes_value["nextActions"][0],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
    assert!(recipes_value["opportunities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|opportunity| opportunity["id"] == "benchmark_freshness"));

    let opportunities =
        handle_opportunities(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let opportunities_value: Value = serde_json::from_str(&opportunities).unwrap();
    assert_eq!(
        opportunities_value["opportunities"][0]["id"],
        "benchmark_freshness"
    );
}

#[test]
fn benchmark_status_ages_ready_evidence_by_oldest_required_preset() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let now = Utc::now();

    write_benchmark_status_test_artifact(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        now - chrono::Duration::days(2),
        "cargo-test",
        "cargo-test",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        now + chrono::Duration::seconds(1),
        "preflight-quick",
        "preflight-quick",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990103T000000Z-product-selftest.json",
        now + chrono::Duration::seconds(2),
        "selftest",
        "selftest",
        "passed",
    );
    write_benchmark_status_test_artifact(
        dir.path(),
        "20990104T000000Z-product-scorecard.json",
        now + chrono::Duration::seconds(3),
        "scorecard",
        "scorecard",
        "passed",
    );

    let status = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&status).unwrap();

    assert_eq!(value["status"], "ready");
    assert_eq!(value["freshness"]["status"], "aging");
    assert_eq!(value["freshness"]["oldestRequiredAge"], "2d");
    assert_eq!(value["freshness"]["latestMeaningfulAge"], "0s");
    assert_eq!(
        value["nextActions"][0],
        SCORECARD_BENCHMARK_REMEDIATION_ACTION
    );
}

#[test]
fn benchmark_cleanup_previews_and_deletes_old_artifacts() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let now = Utc::now();

    let newest_path = write_benchmark_status_test_artifact(
        dir.path(),
        "20990103T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(3),
        "cargo-test",
        "cargo-test",
        "passed",
    );
    let old_path = write_benchmark_status_test_artifact(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        now + chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
    );
    let oldest_path = write_benchmark_status_test_artifact(
        dir.path(),
        "20990101T000000Z-product-scorecard.json",
        now + chrono::Duration::seconds(1),
        "scorecard",
        "scorecard",
        "failed",
    );

    let dry_run = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "clean".into(),
            "--json".into(),
            "--dry-run".into(),
            "--keep".into(),
            "1".into(),
        ],
    )
    .unwrap();
    let dry_value: Value = serde_json::from_str(&dry_run).unwrap();
    assert_eq!(dry_value["schema"], "deepcli.benchmark.cleanup.v1");
    assert_eq!(dry_value["status"], "planned");
    assert_eq!(dry_value["dryRun"], true);
    assert_eq!(dry_value["artifactCount"], 3);
    assert_eq!(dry_value["candidateCount"], 2);
    assert_eq!(dry_value["deletedCount"], 0);
    assert_eq!(dry_value["summary"]["status"], "planned");
    assert_eq!(dry_value["summary"]["dryRun"], true);
    assert_eq!(dry_value["summary"]["artifactCount"], 3);
    assert_eq!(dry_value["summary"]["candidateCount"], 2);
    assert_eq!(dry_value["summary"]["deletedCount"], 0);
    assert_eq!(dry_value["summary"]["keep"], 1);
    assert_eq!(dry_value["summary"]["olderThanDays"], Value::Null);
    assert_eq!(dry_value["summary"]["willDelete"], false);
    assert_eq!(
        dry_value["summary"]["recommendedAction"],
        dry_value["checklist"][0]["command"]
    );
    assert_eq!(
        dry_value["summary"]["recommendedActionLabel"],
        dry_value["checklist"][0]["label"]
    );
    assert_eq!(dry_value["candidates"][0]["artifactPath"], old_path);
    assert_eq!(dry_value["candidates"][1]["artifactPath"], oldest_path);
    assert!(dry_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action
            .as_str()
            .unwrap()
            .contains("benchmark clean --force --keep 1")));
    assert_benchmark_checklist_matches_next_actions(&dry_value);
    assert!(dry_value["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["label"] == "Delete benchmark artifacts"
                && item["command"] == "deepcli benchmark clean --force --keep 1"
        }));
    assert!(dir.path().join(&newest_path).exists());
    assert!(dir.path().join(&old_path).exists());
    assert!(dir.path().join(&oldest_path).exists());

    let forced = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "clean".into(),
            "--json".into(),
            "--keep".into(),
            "1".into(),
            "--force".into(),
        ],
    )
    .unwrap();
    let forced_value: Value = serde_json::from_str(&forced).unwrap();
    assert_eq!(forced_value["status"], "deleted");
    assert_eq!(forced_value["dryRun"], false);
    assert_eq!(forced_value["deletedCount"], 2);
    assert_eq!(forced_value["summary"]["status"], "deleted");
    assert_eq!(forced_value["summary"]["dryRun"], false);
    assert_eq!(forced_value["summary"]["candidateCount"], 2);
    assert_eq!(forced_value["summary"]["deletedCount"], 2);
    assert_eq!(forced_value["summary"]["willDelete"], true);
    assert_eq!(
        forced_value["summary"]["recommendedAction"],
        forced_value["checklist"][0]["command"]
    );
    assert_eq!(
        forced_value["summary"]["recommendedActionLabel"],
        forced_value["checklist"][0]["label"]
    );
    assert_benchmark_checklist_matches_next_actions(&forced_value);
    assert!(dir.path().join(&newest_path).exists());
    assert!(!dir.path().join(&old_path).exists());
    assert!(!dir.path().join(&oldest_path).exists());

    let status = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let status_value: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status_value["artifactCount"], 1);
    assert_eq!(status_value["latestArtifact"]["artifactPath"], newest_path);

    let empty = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "clean".into(),
            "--json".into(),
            "--keep".into(),
            "20".into(),
        ],
    )
    .unwrap();
    let empty_value: Value = serde_json::from_str(&empty).unwrap();
    assert_eq!(empty_value["status"], "empty");
    assert_eq!(empty_value["candidateCount"], 0);
    assert_benchmark_checklist_matches_next_actions(&empty_value);

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["clean".into(), "--output".into(), "../cleanup.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../cleanup.json").exists());
}

#[test]
fn benchmark_show_latest_missing_artifact_suggests_executable_commands() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let error = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["show".into(), "latest".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("no benchmark artifacts found under .deepcli/benchmarks"));
    assert!(error.contains("deepcli benchmark run-suite --json --fail-on-command"));
    assert!(!error.contains("run `/benchmark"));
}

#[test]
fn benchmark_record_list_show_and_scorecard_are_structured() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let record = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "record".into(),
            "--json".into(),
            "--suite".into(),
            "product".into(),
            "--case".into(),
            "scorecard".into(),
            "--command".into(),
            "cargo test".into(),
            "--notes".into(),
            "local product loop".into(),
        ],
    )
    .unwrap();
    let record_value: Value = serde_json::from_str(&record).unwrap();
    assert_eq!(record_value["schema"], BENCHMARK_ARTIFACT_SCHEMA);
    assert_eq!(record_value["suite"], "product");
    assert_eq!(record_value["case"], "scorecard");
    assert_eq!(
        record_value["scorecard"]["schema"],
        "deepcli.scorecard.summary.v1"
    );
    assert_eq!(record_value["execution"]["ranByDeepcli"], false);
    assert_benchmark_checklist_matches_next_actions(&record_value);
    let artifact_path = record_value["artifactPath"].as_str().unwrap();
    assert!(artifact_path.starts_with(".deepcli/benchmarks/"));
    assert!(dir.path().join(artifact_path).exists());
    assert_eq!(record_value["summary"]["status"], "recorded");
    assert_eq!(record_value["summary"]["suite"], "product");
    assert_eq!(record_value["summary"]["case"], "scorecard");
    assert_eq!(record_value["summary"]["preset"], Value::Null);
    assert_eq!(record_value["summary"]["artifactPath"], artifact_path);
    assert_eq!(record_value["summary"]["mode"], "record_only");
    assert_eq!(record_value["summary"]["ranByDeepcli"], false);
    assert_eq!(record_value["summary"]["commandCount"], 1);
    assert_eq!(record_value["summary"]["durationMs"], Value::Null);
    assert_eq!(
        record_value["summary"]["recommendedAction"],
        record_value["checklist"][0]["command"]
    );
    assert_eq!(
        record_value["summary"]["recommendedActionLabel"],
        record_value["checklist"][0]["label"]
    );

    let list = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["list".into(), "--json".into()],
    )
    .unwrap();
    let list_value: Value = serde_json::from_str(&list).unwrap();
    assert_eq!(list_value["schema"], "deepcli.benchmark.list.v1");
    assert_eq!(list_value["artifactCount"], 1);
    assert_eq!(list_value["summary"]["status"], "ok");
    assert_eq!(list_value["summary"]["artifactCount"], 1);
    assert_eq!(list_value["summary"]["latestArtifactPath"], artifact_path);
    assert_eq!(list_value["summary"]["latestSuite"], "product");
    assert_eq!(list_value["summary"]["latestCase"], "scorecard");
    assert_eq!(list_value["summary"]["latestPreset"], Value::Null);
    assert_eq!(list_value["summary"]["latestStatus"], "recorded");
    assert_eq!(
        list_value["summary"]["latestCreatedAt"],
        record_value["createdAt"]
    );
    assert_eq!(
        list_value["summary"]["recommendedAction"],
        list_value["checklist"][0]["command"]
    );
    assert_eq!(
        list_value["summary"]["recommendedActionLabel"],
        list_value["checklist"][0]["label"]
    );
    assert_eq!(list_value["artifacts"][0]["artifactPath"], artifact_path);
    assert!(list_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action
            .as_str()
            .unwrap()
            .contains("benchmark summary --json")));
    assert!(list_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action
            .as_str()
            .unwrap()
            .contains("benchmark clean --dry-run")));
    assert_benchmark_checklist_matches_next_actions(&list_value);

    let show = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["show".into(), "latest".into(), "--json".into()],
    )
    .unwrap();
    let show_value: Value = serde_json::from_str(&show).unwrap();
    assert_eq!(show_value["artifactPath"], artifact_path);
    assert_eq!(show_value["summary"]["status"], "recorded");
    assert_eq!(show_value["summary"]["artifactPath"], artifact_path);
    assert_eq!(
        show_value["summary"]["recommendedAction"],
        show_value["checklist"][0]["command"]
    );
    assert_eq!(
        show_value["summary"]["recommendedActionLabel"],
        show_value["checklist"][0]["label"]
    );
    assert_benchmark_checklist_matches_next_actions(&show_value);

    let scorecard =
        handle_scorecard(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let scorecard_value: Value = serde_json::from_str(&scorecard).unwrap();
    assert!(scorecard_value["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .all(|gap| !gap
            .as_str()
            .unwrap()
            .contains("no local benchmark artifact found")));
}

#[test]
fn benchmark_summary_aggregates_history() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--json".into(),
            "--suite".into(),
            "local".into(),
            "--case".into(),
            "echo".into(),
            "--command".into(),
            "printf ok".into(),
            "--timeout".into(),
            "5".into(),
        ],
    )
    .unwrap();
    handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "run".into(),
            "--json".into(),
            "--suite".into(),
            "local".into(),
            "--case".into(),
            "echo".into(),
            "--command".into(),
            "exit 4".into(),
            "--timeout".into(),
            "5".into(),
        ],
    )
    .unwrap();
    handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "record".into(),
            "--json".into(),
            "--suite".into(),
            "product".into(),
            "--case".into(),
            "scorecard".into(),
        ],
    )
    .unwrap();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["summary".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.benchmark.summary.v1");
    assert_eq!(value["artifactCount"], 3);
    assert_eq!(value["caseCount"], 2);
    assert_eq!(value["summary"]["status"], "ok");
    assert_eq!(value["summary"]["artifactCount"], 3);
    assert_eq!(value["summary"]["caseCount"], 2);
    assert_eq!(value["summary"]["executableCount"], 2);
    assert_eq!(value["summary"]["passedCount"], 1);
    assert_eq!(value["summary"]["failedCount"], 1);
    assert_eq!(value["summary"]["recordedCount"], 1);
    assert_eq!(value["summary"]["passRatePercent"], 50);
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["checklist"][0]["command"]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli benchmark summary"));
    assert!(value["report"].as_str().unwrap().contains("cases:"));
    assert_eq!(value["totals"]["total"], 3);
    assert_eq!(value["totals"]["executableCount"], 2);
    assert_eq!(value["totals"]["passedCount"], 1);
    assert_eq!(value["totals"]["failedCount"], 1);
    assert_eq!(value["totals"]["recordedCount"], 1);
    assert_eq!(value["totals"]["passRatePercent"], 50);
    let cases = value["cases"].as_array().unwrap();
    let local_echo = cases
        .iter()
        .find(|case| case["suite"] == "local" && case["case"] == "echo")
        .unwrap();
    assert_eq!(local_echo["total"], 2);
    assert_eq!(local_echo["executableCount"], 2);
    assert_eq!(local_echo["passedCount"], 1);
    assert_eq!(local_echo["failedCount"], 1);
    assert_eq!(local_echo["passRatePercent"], 50);
    assert!(local_echo["duration"]["averageMs"].is_u64());
    assert!(local_echo["duration"]["minMs"].is_u64());
    assert!(local_echo["duration"]["maxMs"].is_u64());
    assert_eq!(local_echo["latest"]["status"], "failed");
    assert!(local_echo["latest"]["artifactPath"]
        .as_str()
        .unwrap()
        .starts_with(".deepcli/benchmarks/"));

    let product_scorecard = cases
        .iter()
        .find(|case| case["suite"] == "product" && case["case"] == "scorecard")
        .unwrap();
    assert_eq!(product_scorecard["recordedCount"], 1);
    assert!(product_scorecard["passRatePercent"].is_null());

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "summary".into(),
            "--output".into(),
            "../summary.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../summary.json").exists());
}

#[test]
fn benchmark_compare_reports_baseline_status_and_duration_delta() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let now = Utc::now();

    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
        120,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        now + chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "failed",
        250,
    );
    fs::create_dir_all(dir.path().join(".deepcli/baselines")).unwrap();
    fs::write(
        dir.path().join(".deepcli/baselines/competitor.json"),
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "competitor",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": "passed",
                    "durationMs": 150
                },
                {
                    "suite": "product",
                    "case": "preflight-quick",
                    "status": "passed",
                    "durationMs": 200
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--json".into(),
            "--baseline".into(),
            ".deepcli/baselines/competitor.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.benchmark.compare.v1");
    assert_eq!(value["baseline"]["name"], "competitor");
    assert_eq!(value["artifactCount"], 2);
    assert_eq!(value["comparisonCount"], 2);
    assert_eq!(value["status"], "regression");

    let comparisons = value["comparisons"].as_array().unwrap();
    let cargo = comparisons
        .iter()
        .find(|case| case["case"] == "cargo-test")
        .unwrap();
    assert_eq!(cargo["statusComparison"], "same_pass");
    assert_eq!(cargo["durationDeltaMs"], -30);
    assert_eq!(cargo["durationComparison"], "faster");

    let preflight = comparisons
        .iter()
        .find(|case| case["case"] == "preflight-quick")
        .unwrap();
    assert_eq!(preflight["statusComparison"], "regressed");
    assert_eq!(preflight["durationDeltaMs"], 50);
    assert_eq!(preflight["durationComparison"], "slower");
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains("benchmark trends --json")));

    let text = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--baseline".into(),
            ".deepcli/baselines/competitor.json".into(),
        ],
    )
    .unwrap();
    assert!(text.contains("deepcli benchmark compare"));
    assert!(text.contains("status_comparison=regressed"));

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--baseline".into(),
            "../competitor.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
}

#[test]
fn benchmark_baseline_template_writes_compare_ready_json() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "baseline-template".into(),
            "--json".into(),
            "--name".into(),
            "deepcli-main".into(),
            "--output".into(),
            ".deepcli/baselines/deepcli-main.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.benchmark.baseline.v1");
    assert_eq!(value["name"], "deepcli-main");
    assert_eq!(value["status"], "needs_values");
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| {
            action.as_str().unwrap().contains(
                "edit status and durationMs values in .deepcli/baselines/deepcli-main.json",
            )
        }));
    assert!(value["nextActions"].as_array().unwrap().iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/deepcli-main.json --json"
        }));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
    assert!(value["checklist"].as_array().unwrap().iter().any(|item| {
            item["label"] == "Compare benchmark baseline"
                && item["command"]
                    == "deepcli benchmark compare --baseline .deepcli/baselines/deepcli-main.json --json"
        }));
    assert!(!value["checklist"].as_array().unwrap().iter().any(|item| {
        item["command"]
            .as_str()
            .is_some_and(|command| command.starts_with("edit status"))
    }));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("wrote baseline template: .deepcli/baselines/deepcli-main.json"));

    let cases = value["cases"].as_array().unwrap();
    assert!(cases.iter().any(|case| {
        case["preset"] == "cargo-test"
            && case["suite"] == "product"
            && case["case"] == "cargo-test"
            && case["command"] == "cargo test"
            && case["status"].is_null()
            && case["durationMs"].is_null()
    }));
    assert!(cases.iter().any(|case| case["preset"] == "preflight-quick"));
    assert!(cases.iter().any(|case| case["preset"] == "selftest"));
    assert!(cases.iter().any(|case| case["preset"] == "scorecard"));

    let written =
        fs::read_to_string(dir.path().join(".deepcli/baselines/deepcli-main.json")).unwrap();
    let written_value: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(written_value, value);

    let compare = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--json".into(),
            "--baseline".into(),
            ".deepcli/baselines/deepcli-main.json".into(),
        ],
    )
    .unwrap();
    let compare_value: Value = serde_json::from_str(&compare).unwrap();
    assert_eq!(compare_value["baseline"]["name"], "deepcli-main");
    assert_eq!(compare_value["comparisonCount"], 4);
    assert_eq!(compare_value["status"], "incomplete");
    assert!(compare_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains(
            "edit status and durationMs values in .deepcli/baselines/deepcli-main.json"
        )));

    let compare_text = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--baseline".into(),
            ".deepcli/baselines/deepcli-main.json".into(),
        ],
    )
    .unwrap();
    assert!(compare_text
        .contains("edit status and durationMs values in .deepcli/baselines/deepcli-main.json"));

    let no_baseline = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["compare".into(), "--json".into()],
    )
    .unwrap();
    let no_baseline_value: Value = serde_json::from_str(&no_baseline).unwrap();
    assert!(no_baseline_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action
            .as_str()
            .unwrap()
            .contains("benchmark baseline-template --output")));

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "baseline-template".into(),
            "--json".into(),
            "--output".into(),
            "../baseline.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
}

#[test]
fn benchmark_baseline_template_can_capture_current_artifacts() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let now = Utc::now();

    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20980101T000000Z-product-cargo-test.json",
        now - chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
        999,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
        120,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        now + chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
        250,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990103T000000Z-product-selftest.json",
        now + chrono::Duration::seconds(3),
        "selftest",
        "selftest",
        "passed",
        30,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990104T000000Z-product-scorecard.json",
        now + chrono::Duration::seconds(4),
        "scorecard",
        "scorecard",
        "passed",
        10,
    );

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "baseline-template".into(),
            "--json".into(),
            "--from-current".into(),
            "--name".into(),
            "current-main".into(),
            "--output".into(),
            ".deepcli/baselines/current-main.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.benchmark.baseline.v1");
    assert_eq!(value["name"], "current-main");
    assert_eq!(value["status"], "ready");
    assert!(value["nextActions"].as_array().unwrap().iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
        }));
    assert!(!value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().starts_with("edit status")));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
    assert_eq!(
        value["checklist"][0]["command"].as_str(),
        Some("deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json")
    );
    assert_eq!(
        value["checklist"][0]["label"].as_str(),
        Some("Compare benchmark baseline")
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("source: current benchmark artifacts"));

    let cases = value["cases"].as_array().unwrap();
    let cargo = cases
        .iter()
        .find(|case| case["preset"] == "cargo-test")
        .unwrap();
    assert_eq!(cargo["status"], "passed");
    assert_eq!(cargo["durationMs"], 120);
    assert!(cargo["notes"]
        .as_str()
        .unwrap()
        .contains("captured from .deepcli/benchmarks/20990101T000000Z-product-cargo-test.json"));

    let written =
        fs::read_to_string(dir.path().join(".deepcli/baselines/current-main.json")).unwrap();
    let written_value: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(written_value, value);

    let compare = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--json".into(),
            "--baseline".into(),
            ".deepcli/baselines/current-main.json".into(),
        ],
    )
    .unwrap();
    let compare_value: Value = serde_json::from_str(&compare).unwrap();
    assert_eq!(compare_value["status"], "ok");
    assert_eq!(compare_value["comparisonCount"], 4);
    assert!(!compare_value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().starts_with("edit status")));
}

#[test]
fn benchmark_baseline_template_stdout_only_does_not_compare_missing_file() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    write_round_ready_benchmark_history(dir.path());

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "baseline-template".into(),
            "--json".into(),
            "--from-current".into(),
            "--name".into(),
            "current-main".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.benchmark.baseline.v1");
    assert_eq!(value["status"], "ready");
    assert!(!dir
        .path()
        .join(".deepcli/baselines/current-main.json")
        .exists());
    assert_eq!(
            value["nextActions"][0],
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        );
    assert!(!value["nextActions"].as_array().unwrap().iter().any(|action| {
            action.as_str().unwrap()
                == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
        }));
    assert_eq!(
            value["checklist"][0]["command"],
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        );
}

#[test]
fn benchmark_baselines_lists_local_baseline_readiness() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let baselines_dir = dir.path().join(".deepcli/baselines");
    fs::create_dir_all(&baselines_dir).unwrap();
    fs::write(
        baselines_dir.join("current-main.json"),
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "current-main",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": "passed",
                    "durationMs": 120
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        baselines_dir.join("competitor.json"),
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "competitor",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": null,
                    "durationMs": null
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["baselines".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.benchmark.baselines.v1");
    assert_eq!(value["status"], "mixed");
    assert_eq!(value["baselineCount"], 2);
    assert_eq!(value["readyCount"], 1);
    assert_eq!(value["needsValuesCount"], 1);
    assert_eq!(value["defaultBaseline"]["present"], true);
    assert_eq!(value["defaultBaseline"]["status"], "needs_values");

    let baselines = value["baselines"].as_array().unwrap();
    let current = baselines
        .iter()
        .find(|baseline| baseline["path"] == ".deepcli/baselines/current-main.json")
        .unwrap();
    assert_eq!(current["status"], "ready");
    assert_eq!(current["readyToCompare"], true);
    assert_eq!(current["caseCount"], 1);

    let competitor = baselines
        .iter()
        .find(|baseline| baseline["path"] == ".deepcli/baselines/competitor.json")
        .unwrap();
    assert_eq!(competitor["status"], "needs_values");
    assert_eq!(competitor["readyToCompare"], false);
    assert_eq!(competitor["missingValueCount"], 1);

    let next_actions = json_string_array(&value["nextActions"]);
    assert!(next_actions.iter().any(|action| {
        action == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
    }));
    assert!(next_actions.iter().any(|action| {
        action == "edit status and durationMs values in .deepcli/baselines/competitor.json"
    }));
    assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
    assert!(!value["checklist"].as_array().unwrap().iter().any(|item| {
        item["command"]
            .as_str()
            .is_some_and(|command| command.starts_with("edit status"))
    }));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("default baseline: .deepcli/baselines/competitor.json status=needs_values"));
}

#[test]
fn benchmark_baselines_prioritizes_default_template_when_only_current_is_ready() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let baselines_dir = dir.path().join(".deepcli/baselines");
    fs::create_dir_all(&baselines_dir).unwrap();
    fs::write(
        baselines_dir.join("current-main.json"),
        serde_json::to_string_pretty(&json!({
            "schema": "deepcli.benchmark.baseline.v1",
            "name": "current-main",
            "cases": [
                {
                    "suite": "product",
                    "case": "cargo-test",
                    "status": "passed",
                    "durationMs": 120
                },
                {
                    "suite": "product",
                    "case": "preflight-quick",
                    "status": "passed",
                    "durationMs": 250
                },
                {
                    "suite": "product",
                    "case": "selftest",
                    "status": "passed",
                    "durationMs": 30
                },
                {
                    "suite": "product",
                    "case": "scorecard",
                    "status": "passed",
                    "durationMs": 10
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["baselines".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["schema"], "deepcli.benchmark.baselines.v1");
    assert_eq!(value["status"], "needs_default");
    assert_eq!(value["defaultBaseline"]["present"], false);
    assert_eq!(
        next_actions[0],
        "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
    );
    assert!(!next_actions.iter().any(|action| {
            action
                == "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        }));
    assert!(next_actions.iter().any(|action| {
        action == "deepcli benchmark compare --baseline .deepcli/baselines/current-main.json --json"
    }));
    assert_eq!(
        value["checklist"][0]["command"],
        "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
    );
}

#[test]
fn benchmark_baselines_empty_state_guides_template_creation() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["baselines".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.benchmark.baselines.v1");
    assert_eq!(value["status"], "empty");
    assert_eq!(value["baselineCount"], 0);
    assert_eq!(value["defaultBaseline"]["present"], false);
    assert_eq!(value["summary"]["status"], "empty");
    assert_eq!(value["summary"]["baselineCount"], 0);
    assert_eq!(value["summary"]["compareReady"], false);
    assert_eq!(value["summary"]["defaultBaselineStatus"], "missing");
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["nextActions"][0]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    let next_actions = json_string_array(&value["nextActions"]);
    assert!(next_actions.iter().any(|action| {
            action == "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
        }));
    assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);
}

#[test]
fn benchmark_json_reports_expose_executable_action_checklists() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let now = Utc::now();

    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
        120,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990102T000000Z-product-preflight-quick.json",
        now + chrono::Duration::seconds(2),
        "preflight-quick",
        "preflight-quick",
        "passed",
        250,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990103T000000Z-product-selftest.json",
        now + chrono::Duration::seconds(3),
        "selftest",
        "selftest",
        "passed",
        30,
    );
    write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990104T000000Z-product-scorecard.json",
        now + chrono::Duration::seconds(4),
        "scorecard",
        "scorecard",
        "passed",
        10,
    );

    let status = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["status".into(), "--json".into()],
    )
    .unwrap();
    let status_value: Value = serde_json::from_str(&status).unwrap();
    let status_next_actions = json_string_array(&status_value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(&status_value, &status_next_actions);
    assert!(status_value["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["label"] == "Run benchmark suite"
                && item["command"] == "deepcli benchmark run-suite --json --fail-on-command"
        }));

    let summary = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["summary".into(), "--json".into()],
    )
    .unwrap();
    let summary_value: Value = serde_json::from_str(&summary).unwrap();
    let summary_next_actions = json_string_array(&summary_value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(&summary_value, &summary_next_actions);
    assert!(summary_value["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["label"] == "Run benchmark suite"
                && item["command"] == "deepcli benchmark run-suite --json --fail-on-command"
        }));

    let trends = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["trends".into(), "--json".into()],
    )
    .unwrap();
    let trends_value: Value = serde_json::from_str(&trends).unwrap();
    let trends_next_actions = json_string_array(&trends_value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(&trends_value, &trends_next_actions);
    assert!(trends_value["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["label"] == "Refresh benchmark evidence"
                && item["command"] == "deepcli round --json --run-benchmark --fail-on-command"
        }));

    let baseline = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "baseline-template".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/baselines/competitor.json".into(),
        ],
    )
    .unwrap();
    let baseline_value: Value = serde_json::from_str(&baseline).unwrap();
    assert_eq!(baseline_value["status"], "needs_values");
    let baseline_next_actions = json_string_array(&baseline_value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(&baseline_value, &baseline_next_actions);

    let compare = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "compare".into(),
            "--json".into(),
            "--baseline".into(),
            ".deepcli/baselines/competitor.json".into(),
        ],
    )
    .unwrap();
    let compare_value: Value = serde_json::from_str(&compare).unwrap();
    let compare_next_actions = json_string_array(&compare_value["nextActions"]);
    assert!(compare_next_actions
        .iter()
        .any(|action| action.starts_with("edit status and durationMs values")));
    assert_benchmark_checklist_matches_executable_actions(&compare_value, &compare_next_actions);
    assert!(!compare_value["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["command"]
                .as_str()
                .is_some_and(|command| command.starts_with("edit status"))
        }));
}

#[test]
fn benchmark_trends_report_status_and_duration_regressions() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let now = Utc::now();

    let oldest_path = write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990101T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(1),
        "cargo-test",
        "cargo-test",
        "passed",
        100,
    );
    let previous_path = write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990102T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(2),
        "cargo-test",
        "cargo-test",
        "passed",
        120,
    );
    let latest_path = write_benchmark_status_test_artifact_with_duration(
        dir.path(),
        "20990103T000000Z-product-cargo-test.json",
        now + chrono::Duration::seconds(3),
        "cargo-test",
        "cargo-test",
        "failed",
        180,
    );

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "trends".into(),
            "--json".into(),
            "--limit".into(),
            "2".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.benchmark.trends.v1");
    assert_eq!(value["status"], "regression");
    assert_eq!(value["artifactCount"], 3);
    assert_eq!(value["caseCount"], 1);
    assert_eq!(value["recentLimit"], 2);
    assert_eq!(value["summary"]["status"], "regression");
    assert_eq!(value["summary"]["artifactCount"], 3);
    assert_eq!(value["summary"]["caseCount"], 1);
    assert_eq!(value["summary"]["regressionCount"], 1);
    assert_eq!(value["summary"]["slowerCount"], 1);
    assert_eq!(
        value["summary"]["recommendedAction"],
        value["nextActions"][0]
    );
    assert_eq!(
        value["summary"]["recommendedActionLabel"],
        value["checklist"][0]["label"]
    );
    let trend = &value["trends"][0];
    assert_eq!(trend["suite"], "product");
    assert_eq!(trend["case"], "cargo-test");
    assert_eq!(trend["total"], 3);
    assert_eq!(trend["executableCount"], 3);
    assert_eq!(trend["passedCount"], 2);
    assert_eq!(trend["failedCount"], 1);
    assert_eq!(trend["passRatePercent"], 67);
    assert_eq!(trend["statusTrend"], "regressed");
    assert_eq!(trend["durationTrend"], "slower");
    assert_eq!(trend["durationDeltaMs"], 60);
    assert_eq!(trend["latest"]["artifactPath"], latest_path);
    assert_eq!(trend["previous"]["artifactPath"], previous_path);
    assert_eq!(trend["recent"].as_array().unwrap().len(), 2);
    assert_eq!(trend["recent"][0]["artifactPath"], latest_path);
    assert_eq!(trend["recent"][1]["artifactPath"], previous_path);
    assert!(oldest_path.ends_with("cargo-test.json"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action
            .as_str()
            .unwrap()
            .contains("benchmark summary --json")));

    let text = handle_benchmark(dir.path(), &config, &registry, vec!["trend".into()]).unwrap();
    assert!(text.contains("deepcli benchmark trends"));
    assert!(text.contains("status_trend=regressed"));
    assert!(text.contains("duration_trend=slower"));
    assert!(text.contains("duration_delta=60ms"));

    let single_dir = tempdir().unwrap();
    write_benchmark_status_test_artifact_with_duration(
        single_dir.path(),
        "20990104T000000Z-product-selftest.json",
        now + chrono::Duration::seconds(4),
        "selftest",
        "selftest",
        "passed",
        25,
    );
    let single_text =
        handle_benchmark(single_dir.path(), &config, &registry, vec!["trend".into()]).unwrap();
    assert!(single_text.contains("duration_delta=n/a"));
    assert!(!single_text.contains("duration_delta=n/ams"));

    let single_json = handle_benchmark(
        single_dir.path(),
        &config,
        &registry,
        vec!["trends".into(), "--json".into()],
    )
    .unwrap();
    let single_value: Value = serde_json::from_str(&single_json).unwrap();
    assert_eq!(single_value["status"], "insufficient_history");
    let single_next_actions = single_value["nextActions"].as_array().unwrap();
    assert_eq!(
        single_next_actions.first().unwrap().as_str().unwrap(),
        "deepcli round --json --run-benchmark --fail-on-command"
    );
    assert!(single_next_actions
        .iter()
        .any(|action| action.as_str().unwrap()
            == "deepcli benchmark run-suite --json --fail-on-command"));
    assert!(single_value["report"]
        .as_str()
        .unwrap()
        .contains("status: insufficient_history"));

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["trends".into(), "--output".into(), "../trends.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../trends.json").exists());
}

#[test]
fn benchmark_trends_uses_baseline_state_for_followup_actions() {
    let dir = tempdir().unwrap();
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let output = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["trends".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = json_string_array(&value["nextActions"]);

    assert_eq!(value["status"], "ok");
    assert!(next_actions.contains(
            &"deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
                .to_string()
        ));
    assert!(next_actions.contains(
        &"deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json"
            .to_string()
    ));
    assert!(!next_actions.contains(
        &"deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
            .to_string()
    ));
    assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);

    let text = handle_benchmark(dir.path(), &config, &registry, vec!["trends".into()]).unwrap();
    assert!(text.contains(
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json"
        ));
    assert!(!text.contains(
        "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json"
    ));
}

#[test]
fn benchmark_exploration_reports_use_baseline_state_for_followup_actions() {
    let dir = tempdir().unwrap();
    write_round_ready_benchmark_history(dir.path());
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let current_capture =
            "deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json";
    let competitor_template =
        "deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json";
    let competitor_compare =
        "deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json";

    for command in ["presets", "list", "summary"] {
        let output = handle_benchmark(
            dir.path(),
            &config,
            &registry,
            vec![command.into(), "--json".into()],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let next_actions = json_string_array(&value["nextActions"]);

        assert!(
            next_actions.contains(&current_capture.to_string()),
            "{command} should offer current baseline capture before compare"
        );
        assert!(
            next_actions.contains(&competitor_template.to_string()),
            "{command} should offer competitor baseline template before compare"
        );
        assert!(
            !next_actions.contains(&competitor_compare.to_string()),
            "{command} should not offer compare before the default baseline exists"
        );
        assert_benchmark_checklist_matches_executable_actions(&value, &next_actions);

        let text = handle_benchmark(dir.path(), &config, &registry, vec![command.into()]).unwrap();
        assert!(
            text.contains(current_capture),
            "{command} text should offer current baseline capture before compare"
        );
        assert!(
            text.contains(competitor_template),
            "{command} text should offer competitor baseline template before compare"
        );
        assert!(
            !text.contains(competitor_compare),
            "{command} text should not offer compare before the default baseline exists"
        );
    }
}

#[test]
fn benchmark_preserves_scorecard_compatibility_and_output_safety() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();

    let scorecard =
        handle_benchmark(dir.path(), &config, &registry, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&scorecard).unwrap();
    assert_eq!(value["schema"], "deepcli.scorecard.v1");

    let failure = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec!["--json".into(), "--fail-below".into(), "100".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(failure.contains("deepcli.scorecard.v1"));

    let traversal = handle_benchmark(
        dir.path(),
        &config,
        &registry,
        vec![
            "record".into(),
            "--output".into(),
            "../benchmark.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../benchmark.json").exists());
}

#[test]
fn selftest_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".deepcli/credentials")).unwrap();
    fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
    fs::write(dir.path().join(".deepcli/config.json"), "{}").unwrap();
    fs::write(
        dir.path()
            .join(".deepcli/credentials/deepseek-credentials.json"),
        r#"{"apiKey":"sk-selftest-secret","model":"deepseek-v4-pro"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"selftest-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        dir.path().join(".deepcli/logs/deepcli.log"),
        "provider ok\n",
    )
    .unwrap();
    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.append_message("user", "real task").unwrap();

    let output = handle_selftest(
        dir.path(),
        &AppConfig::default(),
        &ToolRegistry::mvp(),
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/selftest.json".into(),
        ],
    )
    .unwrap();

    assert!(!output.contains("sk-selftest-secret"));
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.selftest.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["ready"], true);
    assert_eq!(value["commands"]["missing"].as_array().unwrap().len(), 0);
    assert!(value["commands"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "/selftest"));
    assert!(value["commands"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "/logs"));
    assert_eq!(value["config"]["projectConfig"]["present"], true);
    assert_eq!(value["gitIdentity"]["status"], "no_git");
    assert_eq!(value["provider"]["apiKey"], "configured");
    assert_eq!(value["sessions"]["total"], 1);
    assert_eq!(value["sessions"]["resumable"], 1);
    assert_eq!(value["logs"]["fileCount"], 1);
    assert_eq!(value["logs"]["latestFile"], "deepcli.log");
    assert_eq!(value["tests"]["count"], 1);
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli accept --json"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "deepcli doctor shell --json"));
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(
        next_actions.iter().all(|item| {
            let action = item.as_str().unwrap();
            action.starts_with("deepcli ")
                || action.starts_with("cargo ")
                || action.starts_with("git ")
        }),
        "selftest JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    assert!(
            next_actions.iter().all(|item| {
                let action = item.as_str().unwrap();
                !action.contains("`/") && !action.starts_with("run `")
            }),
            "selftest JSON nextActions should not require parsing slash-command prose: {next_actions:?}"
        );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("deepcli selftest"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/selftest.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn selftest_fail_on_issues_returns_report_and_rejects_unsafe_output() {
    let dir = tempdir().unwrap();
    let config = test_provider_config(MISSING_TEST_PROVIDER);

    let error = handle_selftest(
        dir.path(),
        &config,
        &ToolRegistry::mvp(),
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/selftest-gate.json".into(),
            "--fail-on-issues".into(),
        ],
    )
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(value["schema"], "deepcli.selftest.v1");
    assert_eq!(value["ready"], false);
    assert!(value["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("project config")));
    assert!(value["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("provider API key")));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/selftest-gate.json")).unwrap();
    assert_eq!(written, exit.output);

    let output_error = handle_selftest(
        dir.path(),
        &config,
        &ToolRegistry::mvp(),
        vec!["--output".into(), "../selftest.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(output_error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../selftest.json").exists());
}

#[test]
fn preflight_dry_run_json_lists_release_checks_without_creating_session() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"preflight-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    run_git(dir.path(), &["init"]);

    let output = handle_preflight(
        dir.path(),
        vec![
            "--dry-run".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/preflight.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.preflight.v1");
    assert_eq!(value["status"], "planned");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["mode"], "full");
    for expected in [
        "format",
        "diff-whitespace",
        "clippy",
        "selftest",
        "doctor",
        "privacy",
        "gate",
    ] {
        assert!(value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == expected && check["status"] == "planned"));
    }
    let checks = value["checks"].as_array().unwrap();
    let checklist = value["checklist"].as_array().unwrap();
    assert_eq!(checklist.len(), checks.len());
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"], checks[index]["command"]);
        assert_eq!(item["status"], checks[index]["status"]);
        assert_eq!(item["required"], checks[index]["required"]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
    assert_eq!(checklist[0]["label"], "Check Rust formatting");
    assert_eq!(checklist[5]["label"], "Run privacy scan");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_eq!(next_actions[0], "deepcli preflight --json");
    assert!(!dir.path().join(".deepcli/sessions").exists());
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/preflight.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn preflight_quick_dry_run_skips_slow_checks_and_rejects_unsafe_output() {
    let dir = tempdir().unwrap();
    let output = handle_preflight(
        dir.path(),
        vec!["--dry-run".into(), "--quick".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["status"], "planned");
    assert_eq!(value["mode"], "quick");
    let commands = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| check["command"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(commands.contains(&"deepcli privacy --json --fail-on-findings --no-history"));
    assert!(!commands.contains(&"deepcli privacy --json --fail-on-findings"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_eq!(next_actions[0], "deepcli preflight --quick --json");
    assert!(value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "gate"
            && check["status"] == "skipped"
            && check["note"] == "skipped by --quick"));

    let error = handle_preflight(
        dir.path(),
        vec!["--output".into(), "../preflight.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../preflight.json").exists());
}

#[test]
fn preflight_json_and_text_surface_runtime_diagnostics() {
    let dir = tempdir().unwrap();
    let checks = vec![
        PreflightCheckResult {
            name: "format".to_string(),
            command: "cargo fmt --check".to_string(),
            status: "passed".to_string(),
            required: true,
            exit_code: Some(0),
            duration_ms: Some(20),
            stdout_chars: 0,
            stderr_chars: 0,
            output: None,
            note: None,
        },
        PreflightCheckResult {
            name: "doctor".to_string(),
            command: "deepcli doctor --quick --json".to_string(),
            status: "passed".to_string(),
            required: true,
            exit_code: Some(0),
            duration_ms: Some(10),
            stdout_chars: 500,
            stderr_chars: 20,
            output: Some("doctor output".to_string()),
            note: None,
        },
        PreflightCheckResult {
            name: "privacy".to_string(),
            command: "deepcli privacy --json --fail-on-findings".to_string(),
            status: "passed".to_string(),
            required: true,
            exit_code: Some(0),
            duration_ms: Some(1_500),
            stdout_chars: 3,
            stderr_chars: 0,
            output: Some("privacy output".to_string()),
            note: None,
        },
        PreflightCheckResult {
            name: "gate".to_string(),
            command: "deepcli gate --json".to_string(),
            status: "failed".to_string(),
            required: true,
            exit_code: Some(1),
            duration_ms: Some(30),
            stdout_chars: 10,
            stderr_chars: 15,
            output: Some("gate failed".to_string()),
            note: None,
        },
    ];
    let options = PreflightOptions::default();
    let next_actions = preflight_next_actions("failed", &checks, &options);
    let report_text = format_preflight_text(dir.path(), "failed", &options, &checks, &next_actions);
    let report = PreflightReport {
        report: report_text.clone(),
        status: "failed".to_string(),
        dry_run: false,
        quick: false,
        fail_fast: false,
        checks,
        next_actions,
    };

    let output = format_preflight_json(dir.path(), &report).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["diagnostics"]["totalDurationMs"], 1_560);
    assert_eq!(value["diagnostics"]["measuredChecks"], 4);
    assert_eq!(value["diagnostics"]["slowestCheck"]["name"], "privacy");
    assert_eq!(value["diagnostics"]["slowestCheck"]["durationMs"], 1_500);
    assert_eq!(value["diagnostics"]["largestOutputCheck"]["name"], "doctor");
    assert_eq!(
        value["diagnostics"]["largestOutputCheck"]["outputChars"],
        520
    );
    assert_eq!(
        value["diagnostics"]["failedRequiredChecks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["gate"]
    );
    assert!(value["report"].as_str().unwrap().contains("diagnostics:"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("slowest=privacy 1500ms"));
    assert!(report_text.contains("largest_output=doctor 520 chars"));
}

#[test]
fn completion_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let output = handle_completion(
        dir.path(),
        vec![
            "json".into(),
            "--output".into(),
            ".deepcli/exports/commands.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.completion.v1");
    assert_eq!(value["program"], "deepcli");
    assert!(value["shells"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap() == "zsh"));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "completion"));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "deepseek"));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "selftest" && item["runningSafe"] == true));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "round" && item["group"] == "core"));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "benchmark" && item["group"] == "support"));
    assert!(value["groups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == "legacy"
            && item["policy"].as_str().unwrap().contains("compatibility")));
    assert!(value["groups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == "support"
            && item["visibility"].as_str().unwrap().contains("support")));
    assert!(value["legacyCommands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "/cleanup"
            && item["successor"] == "/session prune-empty"
            && item["policy"].as_str().unwrap().contains("compatibility")));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "repl" && item["group"] == "support"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/commands.json")).unwrap();
    assert_eq!(written, output);

    let zsh = handle_completion(dir.path(), vec!["zsh".into()]).unwrap();
    assert!(zsh.contains("#compdef deepcli"));
    assert!(zsh.contains("completion"));
    assert!(zsh.contains("deepseek"));

    let guide = handle_completion(dir.path(), Vec::new()).unwrap();
    assert!(guide.contains("deepcli completion"));
    assert!(guide.contains("deepcli completion status zsh"));
    assert!(guide.contains("deepcli completion install zsh --force"));
    assert!(guide.contains("deepcli completion zsh"));
}

#[test]
fn completion_install_dry_run_and_force_are_structured() {
    let home = tempdir().unwrap();
    let script = format_completion_script(CompletionFormat::Zsh, &completion_commands()).unwrap();

    let dry_run =
        install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, false, false)
            .unwrap();
    assert_eq!(dry_run.status, "dry_run");
    assert!(dry_run.dry_run);
    assert!(!dry_run.target_path.exists());
    assert!(dry_run
        .next_actions
        .iter()
        .any(|action| action.contains("--force")));
    assert_executable_deepcli_actions(&dry_run.next_actions);

    let installed =
        install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, true, false)
            .unwrap();
    assert_eq!(installed.status, "installed");
    assert!(!installed.dry_run);
    assert!(installed.parent_created);
    assert_eq!(fs::read_to_string(&installed.target_path).unwrap(), script);
    assert_executable_deepcli_actions(&installed.next_actions);

    let up_to_date =
        install_completion_script_in(home.path(), CompletionFormat::Zsh, &script, true, false)
            .unwrap();
    assert_eq!(up_to_date.status, "up_to_date");
    assert_executable_deepcli_actions(&up_to_date.next_actions);

    let value: Value =
        serde_json::from_str(&format_completion_install_json(&installed).unwrap()).unwrap();
    assert_eq!(value["schema"], "deepcli.completion.install.v1");
    assert_eq!(value["shell"], "zsh");
    assert_eq!(value["status"], "installed");
    assert_eq!(value["dryRun"], false);
    assert!(value["targetPath"]
        .as_str()
        .unwrap()
        .ends_with(".zsh/completions/_deepcli"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Check shell completion".to_string()));
    assert!(checklist_labels.contains(&"Check shell install".to_string()));
}

#[test]
fn completion_status_reports_missing_stale_and_up_to_date() {
    let home = tempdir().unwrap();
    let script = format_completion_script(CompletionFormat::Zsh, &completion_commands()).unwrap();

    let missing = completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
    assert_eq!(missing.status, "missing");
    assert!(!missing.installed);
    assert!(!missing.up_to_date);
    assert!(missing
        .next_actions
        .iter()
        .any(|action| action == "deepcli completion install zsh --force"));
    assert_executable_deepcli_actions(&missing.next_actions);

    let target = completion_install_target(home.path(), CompletionFormat::Zsh).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "old completion").unwrap();
    let stale = completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
    assert_eq!(stale.status, "stale");
    assert!(stale.installed);
    assert!(!stale.up_to_date);
    assert_eq!(stale.installed_bytes, Some("old completion".len()));
    assert!(stale
        .next_actions
        .iter()
        .any(|action| action == "deepcli completion install zsh --force"));
    assert_executable_deepcli_actions(&stale.next_actions);

    fs::write(&target, &script).unwrap();
    let up_to_date =
        completion_status_report_in(home.path(), CompletionFormat::Zsh, &script).unwrap();
    assert_eq!(up_to_date.status, "up_to_date");
    assert!(up_to_date.installed);
    assert!(up_to_date.up_to_date);
    assert_executable_deepcli_actions(&up_to_date.next_actions);

    let value: Value =
        serde_json::from_str(&format_completion_status_json(&up_to_date).unwrap()).unwrap();
    assert_eq!(value["schema"], "deepcli.completion.status.v1");
    assert_eq!(value["shell"], "zsh");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Check shell completion".to_string()));
    assert!(checklist_labels.contains(&"Check shell install".to_string()));
    assert_eq!(value["status"], "up_to_date");
    assert_eq!(value["installed"], true);
    assert_eq!(value["upToDate"], true);
}

#[test]
fn completion_rejects_conflicts_and_unsafe_output() {
    let dir = tempdir().unwrap();
    let conflict = handle_completion(dir.path(), vec!["zsh".into(), "bash".into()])
        .unwrap_err()
        .to_string();
    assert!(conflict.contains("conflicting /completion formats"));

    let output_error = handle_completion(
        dir.path(),
        vec!["json".into(), "--output".into(), "../commands.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(output_error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../commands.json").exists());

    let install_json_error = handle_completion(dir.path(), vec!["install".into(), "json".into()])
        .unwrap_err()
        .to_string();
    assert!(install_json_error.contains("use --json for an install report"));

    let status_json_error = handle_completion(dir.path(), vec!["status".into(), "json".into()])
        .unwrap_err()
        .to_string();
    assert!(status_json_error.contains("use --json for a status report"));

    let status_force_error = handle_completion(
        dir.path(),
        vec!["status".into(), "zsh".into(), "--force".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(status_force_error.contains("does not accept --force"));

    let force_dry_run_error = handle_completion(
        dir.path(),
        vec![
            "install".into(),
            "zsh".into(),
            "--force".into(),
            "--dry-run".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(force_dry_run_error.contains("--force cannot be combined with --dry-run"));
}

#[test]
fn version_command_reports_local_metadata_and_writes_json() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".deepcli")).unwrap();
    fs::write(dir.path().join(".deepcli/config.json"), "{}\n").unwrap();
    let config = AppConfig::default();

    let text = handle_version(dir.path(), &config, Vec::new()).unwrap();
    assert!(text.contains(concat!("deepcli ", env!("CARGO_PKG_VERSION"))));
    assert!(text.contains("project config: .deepcli/config.json (present)"));
    assert!(text.contains("default provider: deepseek"));
    assert!(text.contains("provider turn timeout: 600s"));
    assert!(text.contains("deepcli support"));

    let output = handle_version(
        dir.path(),
        &config,
        vec![
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/version.json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.version.v1");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["projectConfig"]["present"], true);
    assert_eq!(value["defaultProvider"], "deepseek");
    assert_eq!(value["providerTurnTimeoutSeconds"], 600);
    assert!(value["commandCount"].as_u64().unwrap() > 0);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert_eq!(next_actions[0], "deepcli quickstart --check");
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli support"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/version.json")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), value);
}

#[test]
fn version_command_rejects_unknown_options_and_path_traversal() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let unknown = handle_version(dir.path(), &config, vec!["--verbose".to_string()])
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("unsupported /version option"));

    let traversal = handle_version(
        dir.path(),
        &config,
        vec!["--output".to_string(), "../version.json".to_string()],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../version.json").exists());
}

fn test_executor(dir: &Path) -> ToolExecutor {
    let config = AppConfig::default();
    let permissions =
        PermissionEngine::new(dir, config.permissions.clone(), config.sandbox.clone())
            .with_auto_reviewer(config.agent.auto_reviewer);
    ToolExecutor::new(dir, permissions, None, config.agent.max_subagent_depth)
}

#[tokio::test]
async fn agent_list_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let store = AgentStore::new(dir.path());
    let parent = uuid::Uuid::new_v4();
    let task = store
        .create_subagent_task(
            Some(parent),
            "inspect parser",
            1,
            vec![PathBuf::from("src/parser.rs")],
        )
        .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_agent(
        dir.path(),
        &executor,
        vec![
            "list".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/agents.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.agent.inspect.v1");
    assert_eq!(value["kind"], "list");
    assert_eq!(value["agentCount"], 1);
    assert_eq!(value["agents"][0]["id"], task.id.to_string());
    assert_eq!(value["agents"][0]["shortId"], short_id(&task.id));
    assert_eq!(value["agents"][0]["parentSessionId"], parent.to_string());
    assert_eq!(value["agents"][0]["task"], "inspect parser");
    assert_eq!(value["agents"][0]["depth"], 1);
    assert_eq!(value["agents"][0]["writeScope"][0], "src/parser.rs");
    assert_eq!(value["agents"][0]["status"], "queued");
    assert!(value["agents"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with(&format!(".deepcli/agents/tasks/{}.json", task.id)));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect sub-agent".to_string()));
    assert!(checklist_labels.contains(&"List sub-agents".to_string()));
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli agent show {}", short_id(&task.id))));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/agents.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn agent_show_json_output_accepts_short_id_prefix() {
    let dir = tempdir().unwrap();
    let store = AgentStore::new(dir.path());
    let task = store
        .create_subagent_task(None, "inspect parser", 1, Vec::new())
        .unwrap();
    let executor = test_executor(dir.path());
    let short = short_id(&task.id);

    let output = handle_agent(
        dir.path(),
        &executor,
        vec![
            "show".into(),
            short,
            "--json".into(),
            "--output=.deepcli/exports/agent.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.agent.inspect.v1");
    assert_eq!(value["kind"], "show");
    assert_eq!(value["agent"]["id"], task.id.to_string());
    assert_eq!(value["agent"]["task"], "inspect parser");
    assert!(value["report"].as_str().unwrap().contains("inspect parser"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect sub-agent".to_string()));
    assert!(checklist_labels.contains(&"List sub-agents".to_string()));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/agent.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn agent_read_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());
    let error = handle_agent(
        dir.path(),
        &executor,
        vec!["list".into(), "--output".into(), "../agents.json".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../agents.json").exists());
}

#[tokio::test]
async fn agent_logs_json_output_reports_lifecycle_events() {
    let dir = tempdir().unwrap();
    let store = AgentStore::new(dir.path());
    let task = store
        .create_subagent_task(None, "inspect parser", 1, Vec::new())
        .unwrap();
    store
        .mark_subagent_started(task.id, None, Some(123))
        .unwrap();
    store
        .fail_subagent(task.id, "provider credentials missing")
        .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_agent(
        dir.path(),
        &executor,
        vec![
            "logs".into(),
            short_id(&task.id),
            "--json".into(),
            "--output=.deepcli/exports/agent-logs.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.agent.inspect.v1");
    assert_eq!(value["kind"], "logs");
    assert_eq!(value["agent"]["id"], task.id.to_string());
    assert_eq!(value["eventCount"], 3);
    assert_eq!(value["events"][0]["type"], "created");
    assert_eq!(value["events"][1]["type"], "started");
    assert_eq!(value["events"][2]["type"], "failed");
    assert!(value["eventLogPath"]
        .as_str()
        .unwrap()
        .ends_with(&format!(".deepcli/agents/events/{}.jsonl", task.id)));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli agent resume {}", short_id(&task.id))));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/agent-logs.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn agent_run_action_is_not_supported() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());
    let error = handle_agent(
        dir.path(),
        &executor,
        vec!["run".into(), "abc123".into(), "--json".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("unsupported /agent action `run`"));
}

#[tokio::test]
async fn agent_resume_json_uses_runtime_and_persists_failure_for_resume() {
    let dir = tempdir().unwrap();
    let store = AgentStore::new(dir.path());
    let task = store
        .create_subagent_task(None, "inspect parser", 1, Vec::new())
        .unwrap();
    let executor = test_executor(dir.path());
    let mut config = AppConfig {
        default_provider: "missing-subagent-test".to_string(),
        ..AppConfig::default()
    };
    config.providers.insert(
        "missing-subagent-test".to_string(),
        ProviderConfig {
            provider_type: "deepseek".to_string(),
            credentials_file: PathBuf::from(".deepcli/credentials/missing-subagent-test.json"),
            acceptance_model: Some("missing-model".to_string()),
            capabilities: Vec::new(),
        },
    );

    let output = agent::handle_agent_with_config(
        dir.path(),
        &config,
        None,
        &executor,
        vec!["resume".into(), short_id(&task.id), "--json".into()],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.agent.inspect.v1");
    assert_eq!(value["kind"], "resume");
    assert_eq!(value["status"], "failed");
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("apiKey is missing"));
    let loaded = store.load(task.id).unwrap();
    assert_eq!(loaded.status, SubagentStatus::Failed);
    assert_eq!(loaded.attempts, 1);
    assert!(loaded.child_session_id.is_some());
    assert!(loaded
        .last_error
        .as_deref()
        .unwrap()
        .contains("apiKey is missing"));
    let event_types = store
        .read_subagent_events(task.id)
        .unwrap()
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec!["created", "started", "heartbeat", "failed"]
    );
}

fn write_minimal_cargo_project(dir: &Path) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"deepcli-test-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
            dir.join("src/lib.rs"),
            "pub fn ok() -> bool { true }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::ok()); }\n}\n",
        )
        .unwrap();
}

#[tokio::test]
async fn test_discover_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    let executor = test_executor(dir.path());

    let output = handle_test(
        dir.path(),
        &executor,
        vec![
            "discover".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/tests.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.test.inspect.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["kind"], "discover");
    assert_eq!(value["commandCount"], 1);
    assert_eq!(value["commands"][0]["source"], "Cargo.toml");
    assert_eq!(value["commands"][0]["command"], "cargo test");
    assert_eq!(value["commands"][0]["requiresDocker"], false);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli test run --json"));
    assert!(next_actions.iter().any(|action| {
        action.starts_with("deepcli test run --json -- ") && action.contains("cargo test")
    }));
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Run test command".to_string()));
    assert!(checklist_labels.contains(&"Open test help".to_string()));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/tests.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn test_run_json_output_reports_status_and_output() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("Makefile"), "test:\n\t@printf ok\n").unwrap();
    let executor = test_executor(dir.path());

    let output = handle_test(
        dir.path(),
        &executor,
        vec![
            "run".into(),
            "--json".into(),
            "--output=.deepcli/exports/test-run.json".into(),
            "--".into(),
            "make test".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.test.inspect.v1");
    assert_eq!(value["status"], "passed");
    assert_eq!(value["kind"], "run");
    assert_eq!(value["passed"], true);
    assert_eq!(value["command"], "make test");
    assert_eq!(value["exitCode"], 0);
    assert_eq!(value["stdout"], "ok");
    assert_eq!(value["stderr"], "");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli accept --json"));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli gate --json"));
    assert!(next_actions.iter().any(|action| {
        action.starts_with("deepcli test run --json -- ") && action.contains("make test")
    }));
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Run acceptance checks".to_string()));
    assert!(checklist_labels.contains(&"Run delivery gate".to_string()));
    assert!(checklist_labels.contains(&"Run test command".to_string()));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/test-run.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn test_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_test(
        dir.path(),
        &executor,
        vec!["discover".into(), "--output".into(), "../tests.json".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../tests.json").exists());
}

#[tokio::test]
async fn git_status_json_outputs_structured_report_and_rejects_unknown_options() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
    )
    .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_git(
        dir.path(),
        &executor,
        vec!["status".into(), "--json".into()],
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.git.inspect.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["kind"], "status");
    assert_eq!(value["command"], "git status --short");
    assert_eq!(value["exitCode"], 0);
    assert!(value["stdout"].as_str().unwrap().contains("src/lib.rs"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("git status --short"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli git diff --json"));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli git message --json"));
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect git diff".to_string()));
    assert!(checklist_labels.contains(&"Prepare commit message".to_string()));
    assert!(checklist_labels.contains(&"Review current diff".to_string()));

    let error = handle_git(
        dir.path(),
        &executor,
        vec!["status".into(), "--bogus".into()],
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("unsupported /git status option `--bogus`"));
}

#[tokio::test]
async fn git_read_json_output_can_be_written_and_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
    )
    .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_git(
        dir.path(),
        &executor,
        vec![
            "status".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/git-status.json".into(),
        ],
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.git.inspect.v1");
    assert_eq!(value["kind"], "status");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/git-status.json")).unwrap();
    assert_eq!(written, output);

    let error = handle_git(
        dir.path(),
        &executor,
        vec![
            "status".into(),
            "--json".into(),
            "--output=../git.json".into(),
        ],
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../git.json").exists());
}

#[tokio::test]
async fn git_write_actions_reject_unknown_options_before_execution() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
    )
    .unwrap();
    let executor = test_executor(dir.path());

    let branch_error = handle_git(
        dir.path(),
        &executor,
        vec![
            "create-branch".into(),
            "feature/safe".into(),
            "--bogus".into(),
        ],
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(branch_error.contains("unexpected /git create-branch argument `--bogus`"));
    let branches = Command::new("git")
        .args(["branch", "--list", "feature/safe"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(branches.status.success());
    assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(head_before.status.success());
    let commit_error = handle_git(
        dir.path(),
        &executor,
        vec!["commit".into(), "update".into(), "--bogus".into()],
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(commit_error.contains("unexpected /git commit argument `--bogus`"));
    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(head_after.status.success());
    assert_eq!(head_after.stdout, head_before.stdout);
}

#[tokio::test]
async fn git_write_dry_run_json_previews_without_execution() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn ok() -> bool { true }\npub fn changed() -> bool { ok() }\n",
    )
    .unwrap();
    let executor = test_executor(dir.path());

    let branch_output = handle_git(
        dir.path(),
        &executor,
        vec![
            "create-branch".into(),
            "feature/preview".into(),
            "--dry-run".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/git-branch-preview.json".into(),
        ],
    )
    .await
    .unwrap();
    let branch_value: Value = serde_json::from_str(&branch_output).unwrap();
    assert_eq!(branch_value["schema"], "deepcli.git.action.v1");
    assert_eq!(branch_value["status"], "dry_run");
    assert_eq!(branch_value["action"], "create-branch");
    assert_eq!(branch_value["dryRun"], true);
    assert_eq!(branch_value["subject"], "feature/preview");
    assert_eq!(branch_value["command"], "git switch -c feature/preview");
    let branch_next_actions = json_string_array(&branch_value["nextActions"]);
    assert_executable_deepcli_actions(&branch_next_actions);
    assert!(branch_next_actions
        .iter()
        .any(|action| action == "deepcli git create-branch feature/preview"));
    let branch_written =
        fs::read_to_string(dir.path().join(".deepcli/exports/git-branch-preview.json")).unwrap();
    assert_eq!(branch_written, branch_output);
    let branches = Command::new("git")
        .args(["branch", "--list", "feature/preview"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(branches.status.success());
    assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(head_before.status.success());
    let commit_output = handle_git(
        dir.path(),
        &executor,
        vec![
            "commit".into(),
            "preview".into(),
            "checkpoint".into(),
            "--dry-run".into(),
            "--json".into(),
        ],
    )
    .await
    .unwrap();
    let commit_value: Value = serde_json::from_str(&commit_output).unwrap();
    assert_eq!(commit_value["schema"], "deepcli.git.action.v1");
    assert_eq!(commit_value["status"], "dry_run");
    assert_eq!(commit_value["action"], "commit");
    assert_eq!(commit_value["subject"], "preview checkpoint");
    assert_eq!(
        commit_value["command"],
        "git commit-tree <approved-staged-tree> -F - && git update-ref HEAD <new-commit> <old-head>"
    );
    let commit_next_actions = json_string_array(&commit_value["nextActions"]);
    assert_executable_deepcli_actions(&commit_next_actions);
    assert!(commit_next_actions
        .iter()
        .any(|action| action == "deepcli git commit 'preview checkpoint'"));
    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(head_after.status.success());
    assert_eq!(head_after.stdout, head_before.stdout);
}

fn run_git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn expected_git_identity(name: &str, email: &str) -> GitIdentityConfig {
    GitIdentityConfig {
        user_name: Some(name.to_string()),
        user_email: Some(email.to_string()),
    }
}

#[test]
fn git_identity_report_matches_project_expectation() {
    let dir = tempdir().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.name", "zero-kotori"]);
    run_git(
        dir.path(),
        &["config", "user.email", "kotorizero8@gmail.com"],
    );

    let report = build_git_identity_report(
        dir.path(),
        &expected_git_identity("zero-kotori", "kotorizero8@gmail.com"),
    );

    assert_eq!(report.status, "ok");
    assert!(report.issues.is_empty());
    assert_eq!(report.actual_name.as_deref(), Some("zero-kotori"));
    assert_eq!(
        report.actual_email.as_deref(),
        Some("kotorizero8@gmail.com")
    );
}

#[test]
fn git_identity_report_flags_wrong_effective_identity() {
    let dir = tempdir().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.name", "wrong-user"]);
    run_git(dir.path(), &["config", "user.email", "wrong@example.test"]);

    let report = build_git_identity_report(
        dir.path(),
        &expected_git_identity("zero-kotori", "kotorizero8@gmail.com"),
    );

    assert_eq!(report.status, "mismatch");
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.contains("git user.name")));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.contains("git user.email")));
    assert!(report
        .next_actions
        .iter()
        .any(|action| action.contains("git config user.email")));
}

#[test]
fn git_identity_report_skips_global_config_outside_git_repo() {
    let dir = tempdir().unwrap();

    let report = build_git_identity_report(
        dir.path(),
        &expected_git_identity("zero-kotori", "kotorizero8@gmail.com"),
    );

    assert_eq!(report.status, "no_git");
    assert_eq!(report.actual_name, None);
    assert_eq!(report.actual_email, None);
    assert_eq!(
        format_git_identity_summary(&report),
        "not a git repository status=no_git"
    );
}

fn init_git_repo_with_baseline(dir: &Path) {
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "deepcli-test@example.com"]);
    run_git(dir, &["config", "user.name", "deepcli test"]);
    fs::write(dir.join(".gitignore"), "target/\n").unwrap();
    run_git(dir, &["add", "Cargo.toml", "src/lib.rs", ".gitignore"]);
    run_git(dir, &["commit", "-m", "baseline"]);
}

#[test]
fn privacy_scan_reports_history_findings_and_redacts_samples() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"privacy-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let fixture_key = format!("{}{}", "sk-", "test-secret-value");
    let local_path = format!("{USER_HOME_PREFIX}alice/private/repo");
    fs::write(
            dir.path().join("src/lib.rs"),
            format!(
                "pub const LOCAL_PATH: &str = \"{local_path}\";\npub const FAKE_KEY: &str = \"{fixture_key}\";\n",
            ),
        )
        .unwrap();
    fs::write(dir.path().join(".env"), "DEEPSEEK_API_KEY=placeholder\n").unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "person@example.org"]);
    run_git(dir.path(), &["config", "user.name", "privacy tester"]);
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "privacy baseline"]);

    let output = handle_privacy_scan(
        dir.path(),
        &AppConfig::default(),
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/privacy.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["schema"], "deepcli.privacy.scan.v1");
    assert_eq!(value["status"], "high_risk");
    assert!(value["counts"]["high"].as_u64().unwrap() >= 1);
    assert!(value["counts"]["medium"].as_u64().unwrap() >= 1);
    assert!(value["counts"]["low"].as_u64().unwrap() >= 1);
    assert!(output.contains("tracked_sensitive_path"));
    assert!(output.contains("absolute_user_path"));
    assert!(output.contains("secret_shaped_fixture"));
    assert!(output.contains("\"occurrences\""));
    assert!(output.contains(&redacted_user_home()));
    assert!(!output.contains(&local_path));
    assert!(!output.contains(&fixture_key));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/privacy.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn privacy_scan_deduplicates_repeated_history_occurrences() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    let local_path = format!("{USER_HOME_PREFIX}alice/private/repo");
    fs::write(
        dir.path().join("src/lib.rs"),
        format!("pub const LOCAL_PATH: &str = \"{local_path}\";\n"),
    )
    .unwrap();
    run_git(dir.path(), &["init"]);
    run_git(
        dir.path(),
        &["config", "user.email", "deepcli-test@example.com"],
    );
    run_git(dir.path(), &["config", "user.name", "privacy tester"]);
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "baseline"]);
    fs::write(dir.path().join("README.md"), "second commit\n").unwrap();
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-m", "second"]);

    let output =
        handle_privacy_scan(dir.path(), &AppConfig::default(), vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let findings = value["findings"].as_array().unwrap();
    let user_path_findings = findings
        .iter()
        .filter(|finding| finding["category"] == "absolute_user_path")
        .collect::<Vec<_>>();

    assert_eq!(user_path_findings.len(), 1);
    assert_eq!(user_path_findings[0]["occurrences"], 2);
    assert!(value["counts"]["occurrences"].as_u64().unwrap() >= 2);
}

#[test]
fn privacy_scan_fail_on_findings_returns_report_with_exit_code() {
    let dir = tempdir().unwrap();
    let private_email = format!("person@{}", "corp.dev");
    fs::write(
        dir.path().join("README.md"),
        format!("contact {private_email}\n"),
    )
    .unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", &private_email]);
    run_git(dir.path(), &["config", "user.name", "privacy tester"]);
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-m", "metadata"]);

    let error = handle_privacy_scan(
        dir.path(),
        &AppConfig::default(),
        vec!["--fail-on-findings".into()],
    )
    .unwrap_err()
    .downcast::<CommandExit>()
    .unwrap();

    assert_eq!(error.code, 1);
    assert!(error.output.contains("deepcli privacy scan"));
    assert!(error.output.contains("status: needs_review"));
    assert!(error.output.contains("commit_email"));
}

#[test]
fn privacy_scan_suppresses_allowed_commit_email_metadata() {
    let dir = tempdir().unwrap();
    let public_email = format!("zero-kotori@{}", "users.noreply.github.com");
    fs::write(
        dir.path().join("README.md"),
        format!("public contact {public_email}\n"),
    )
    .unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", &public_email]);
    run_git(dir.path(), &["config", "user.name", "zero-kotori"]);
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-m", "metadata"]);

    let mut config = AppConfig::default();
    config.privacy.allowed_emails = vec![public_email.to_string()];
    let output = handle_privacy_scan(dir.path(), &config, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["status"], "ok");
    assert_eq!(value["counts"]["medium"], 0);
    assert_eq!(value["counts"]["actionable"], 0);
    assert_eq!(value["counts"]["suppressed"], 2);
    assert_eq!(value["counts"]["suppressedOccurrences"], 3);
    assert!(value["suppressedFindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["category"] == "commit_email" && finding["occurrences"] == 2));
    assert!(value["suppressedFindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["category"] == "content_email" && finding["occurrences"] == 1));
    assert!(!output.contains(&public_email));
    assert!(output.contains("z***@users.noreply.github.com"));
}

#[test]
fn privacy_scan_suppresses_allowed_user_paths() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".deepcli")).unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    let old_root = format!("{USER_HOME_PREFIX}alice/projects/deepcli");
    let redacted_old_root = format!("{USER_HOME_PREFIX}<user>/projects/deepcli");
    fs::write(
        dir.path().join(".deepcli/config.json"),
        format!("{{\"privacy\":{{\"allowedUserPaths\":[\"{redacted_old_root}\"]}}}}\n"),
    )
    .unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        format!("pub const OLD_ROOT: &str = \"{old_root}/scripts\";\n"),
    )
    .unwrap();
    run_git(dir.path(), &["init"]);
    run_git(
        dir.path(),
        &["config", "user.email", "deepcli-test@example.com"],
    );
    run_git(dir.path(), &["config", "user.name", "privacy tester"]);
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "legacy path"]);

    let mut config = AppConfig::default();
    config.privacy.allowed_user_paths = vec![redacted_old_root.clone()];
    let output = handle_privacy_scan(dir.path(), &config, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["status"], "ok");
    assert_eq!(value["counts"]["medium"], 0);
    assert_eq!(value["counts"]["actionable"], 0);
    let suppressed_user_path_occurrences: u64 = value["suppressedFindings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["category"] == "absolute_user_path")
        .map(|finding| finding["occurrences"].as_u64().unwrap())
        .sum();
    assert_eq!(suppressed_user_path_occurrences, 2);
    assert!(!output.contains("alice"));
    assert!(output.contains(&redacted_old_root));
}

#[test]
fn privacy_scan_flags_configured_blocked_terms_without_leaking_term() {
    let dir = tempdir().unwrap();
    let blocked = "legacy_product_name";
    fs::create_dir_all(dir.path().join(".deepcli")).unwrap();
    fs::write(
        dir.path().join(".deepcli/config.json"),
        format!("{{\"privacy\":{{\"blockedTerms\":[\"{blocked}\"]}}}}\n"),
    )
    .unwrap();
    fs::write(
        dir.path().join("README.md"),
        format!("Use {blocked} as the old command name.\n"),
    )
    .unwrap();
    run_git(dir.path(), &["init"]);
    run_git(
        dir.path(),
        &["config", "user.email", "deepcli-test@example.com"],
    );
    run_git(dir.path(), &["config", "user.name", "privacy tester"]);
    run_git(dir.path(), &["add", "README.md", ".deepcli/config.json"]);
    run_git(
        dir.path(),
        &["commit", "-m", &format!("document {blocked}")],
    );

    let mut config = AppConfig::default();
    config.privacy.blocked_terms = vec![blocked.to_string()];
    let error = handle_privacy_scan(
        dir.path(),
        &config,
        vec!["--json".into(), "--fail-on-findings".into()],
    )
    .unwrap_err()
    .downcast::<CommandExit>()
    .unwrap();
    let value: Value = serde_json::from_str(&error.output).unwrap();

    assert_eq!(error.code, 1);
    assert_eq!(value["status"], "needs_review");
    assert!(value["counts"]["medium"].as_u64().unwrap() >= 1);
    assert!(value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |finding| finding["category"] == "blocked_term" && finding["source"] == "git_metadata"
        ));
    assert!(value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["category"] == "blocked_term"
            && finding["source"] == "git_history_content"));
    assert!(!value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["path"] == ".deepcli/config.json"));
    assert!(error.output.contains("<blocked-term>"));
    assert!(!error.output.contains(blocked));
}

#[test]
fn privacy_scan_suppresses_allowed_blocked_terms() {
    let dir = tempdir().unwrap();
    let blocked = "legacy_product_name";
    fs::write(
        dir.path().join("README.md"),
        format!("Use {blocked} only inside accepted migration docs.\n"),
    )
    .unwrap();
    run_git(dir.path(), &["init"]);
    run_git(
        dir.path(),
        &["config", "user.email", "deepcli-test@example.com"],
    );
    run_git(dir.path(), &["config", "user.name", "privacy tester"]);
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-m", "accepted migration docs"]);

    let mut config = AppConfig::default();
    config.privacy.blocked_terms = vec![blocked.to_string()];
    config.privacy.allowed_terms = vec![blocked.to_string()];
    let output = handle_privacy_scan(dir.path(), &config, vec!["--json".into()]).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(value["status"], "ok");
    assert_eq!(value["counts"]["medium"], 0);
    assert_eq!(value["counts"]["actionable"], 0);
    assert!(value["suppressedFindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["category"] == "blocked_term"
            && finding["source"] == "git_history_content"));
    assert!(output.contains("<blocked-term>"));
    assert!(!output.contains(blocked));
}

#[tokio::test]
async fn prompt_delete_removes_custom_prompt_but_not_builtin() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    handle_prompt(
        dir.path(),
        &executor,
        vec![
            "save".into(),
            "reviewer".into(),
            "Review".into(),
            "diff".into(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        handle_prompt(dir.path(), &executor, vec!["get".into(), "reviewer".into()])
            .await
            .unwrap(),
        "Review diff"
    );
    let deleted = handle_prompt(
        dir.path(),
        &executor,
        vec!["delete".into(), "reviewer".into()],
    )
    .await
    .unwrap();
    assert!(deleted.contains("deleted prompt `reviewer`"));

    let error = handle_prompt(
        dir.path(),
        &executor,
        vec!["delete".into(), "code-review".into()],
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("cannot delete built-in prompt"));
}

#[tokio::test]
async fn prompt_render_expands_file_and_custom_variables() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub fn ok() {}\n").unwrap();
    let store = PromptStore::new(dir.path());
    store
        .save(
            "context",
            "{{task}} {{file}} {{file_content}} {{workspace}} {{branch}}",
        )
        .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_prompt(
        dir.path(),
        &executor,
        vec![
            "render".into(),
            "context".into(),
            "--file".into(),
            "src/lib.rs".into(),
            "task=review".into(),
        ],
    )
    .await
    .unwrap();

    assert!(output.contains("review src/lib.rs pub fn ok()"));
    assert!(output.contains(dir.path().to_str().unwrap()));
}

#[tokio::test]
async fn prompt_list_and_get_json_output_are_structured_and_written() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());
    let store = PromptStore::new(dir.path());
    store
        .save("reviewer", "Review {{file}} for {{task}}")
        .unwrap();

    let list_output = handle_prompt(
        dir.path(),
        &executor,
        vec![
            "list".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/prompts.json".into(),
        ],
    )
    .await
    .unwrap();
    let list_value: Value = serde_json::from_str(&list_output).unwrap();
    assert_eq!(list_value["schema"], "deepcli.prompt.inspect.v1");
    assert_eq!(list_value["kind"], "list");
    assert!(list_value["promptCount"].as_u64().unwrap() >= 4);
    assert!(list_value["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|prompt| {
            prompt["name"] == "reviewer"
                && prompt["source"] == "custom"
                && prompt["bodyPreview"].as_str().unwrap().contains("Review")
        }));
    let list_next_actions = json_string_array(&list_value["nextActions"]);
    assert_executable_deepcli_actions(&list_next_actions);
    assert_checklist_matches_executable_actions(&list_value, &list_next_actions);
    let list_checklist_labels = json_checklist_labels(&list_value);
    assert!(list_checklist_labels.contains(&"Open prompt".to_string()));
    assert!(list_checklist_labels.contains(&"Render prompt".to_string()));
    assert!(list_checklist_labels.contains(&"Open prompt help".to_string()));
    assert!(list_next_actions
        .iter()
        .any(|action| action.starts_with("deepcli prompt render ")));
    assert!(list_next_actions
        .iter()
        .any(|action| action == "deepcli help prompt"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/prompts.json")).unwrap();
    assert_eq!(written, list_output);

    let get_output = handle_prompt(
        dir.path(),
        &executor,
        vec![
            "get".into(),
            "reviewer".into(),
            "--json".into(),
            "--output=.deepcli/exports/reviewer.json".into(),
        ],
    )
    .await
    .unwrap();
    let get_value: Value = serde_json::from_str(&get_output).unwrap();
    assert_eq!(get_value["schema"], "deepcli.prompt.inspect.v1");
    assert_eq!(get_value["kind"], "get");
    assert_eq!(get_value["prompt"]["name"], "reviewer");
    assert_eq!(get_value["prompt"]["source"], "custom");
    assert_eq!(get_value["prompt"]["body"], "Review {{file}} for {{task}}");
    assert_eq!(get_value["report"], "Review {{file}} for {{task}}");
    let get_next_actions = json_string_array(&get_value["nextActions"]);
    assert_executable_deepcli_actions(&get_next_actions);
    assert_checklist_matches_executable_actions(&get_value, &get_next_actions);
    let get_checklist_labels = json_checklist_labels(&get_value);
    assert!(get_checklist_labels.contains(&"Open prompt".to_string()));
    assert!(get_checklist_labels.contains(&"Render prompt".to_string()));
    assert!(get_checklist_labels.contains(&"Open prompt help".to_string()));
    assert!(get_next_actions
        .iter()
        .any(|action| action == "deepcli prompt get reviewer"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/reviewer.json")).unwrap();
    assert_eq!(written, get_output);
}

#[tokio::test]
async fn prompt_render_json_output_includes_context_and_rendered_text() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub fn ok() {}\n").unwrap();
    let store = PromptStore::new(dir.path());
    store
        .save("context", "{{task}} {{file}} {{file_content}}")
        .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_prompt(
        dir.path(),
        &executor,
        vec![
            "render".into(),
            "context".into(),
            "--file".into(),
            "src/lib.rs".into(),
            "task=review".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/rendered-prompt.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.prompt.inspect.v1");
    assert_eq!(value["kind"], "render");
    assert_eq!(value["prompt"]["name"], "context");
    assert_eq!(value["context"]["file"], "src/lib.rs");
    assert_eq!(value["context"]["variables"]["task"], "review");
    assert!(value["rendered"]
        .as_str()
        .unwrap()
        .contains("review src/lib.rs pub fn ok()"));
    assert_eq!(value["report"], value["rendered"]);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Open prompt".to_string()));
    assert!(checklist_labels.contains(&"Render prompt".to_string()));
    assert!(checklist_labels.contains(&"Open prompt help".to_string()));

    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/rendered-prompt.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn prompt_read_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());
    let error = handle_prompt(
        dir.path(),
        &executor,
        vec!["list".into(), "--output".into(), "../prompts.json".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../prompts.json").exists());
}

#[test]
fn skill_list_explains_empty_project_skills() {
    let dir = tempdir().unwrap();

    let output = handle_skill(dir.path(), vec!["list".into()]).unwrap();

    assert!(output.contains("no project skills registered"));
    assert!(output.contains("/skill generate"));
}

#[test]
fn skill_list_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let store = SkillStore::new(dir.path());
    store
        .generate("compiler", "SysY compiler workflow")
        .unwrap();

    let output = handle_skill(
        dir.path(),
        vec![
            "list".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/skills.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.skill.inspect.v1");
    assert_eq!(value["kind"], "list");
    assert_eq!(value["skillCount"], 1);
    assert_eq!(value["skills"][0]["name"], "compiler");
    assert_eq!(value["skills"][0]["description"], "SysY compiler workflow");
    assert_eq!(value["skills"][0]["maxDepth"], 1);
    assert!(value["skills"][0]["metadataPath"]
        .as_str()
        .unwrap()
        .ends_with(".deepcli/skills/compiler/skill.json"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("compiler - SysY compiler workflow"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Run skill".to_string()));
    assert!(checklist_labels.contains(&"List skills".to_string()));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli skill run compiler"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/skills.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn skill_run_json_output_includes_instructions_and_metadata() {
    let dir = tempdir().unwrap();
    let store = SkillStore::new(dir.path());
    store
        .generate("compiler", "SysY compiler workflow")
        .unwrap();

    let output = handle_skill(
        dir.path(),
        vec![
            "run".into(),
            "compiler".into(),
            "--json".into(),
            "--output=.deepcli/exports/compiler-skill.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.skill.inspect.v1");
    assert_eq!(value["kind"], "run");
    assert_eq!(value["skill"]["name"], "compiler");
    assert!(value["instructions"]
        .as_str()
        .unwrap()
        .contains("SysY compiler workflow"));
    assert_eq!(value["report"], value["instructions"]);
    assert!(value["instructionChars"].as_u64().unwrap() > 0);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Run skill".to_string()));
    assert!(checklist_labels.contains(&"List skills".to_string()));

    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/compiler-skill.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn skill_read_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let error = handle_skill(
        dir.path(),
        vec!["list".into(), "--output".into(), "../skills.json".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../skills.json").exists());
}

#[test]
fn parses_session_limit_and_export_path() {
    let (limit, id, explicit) = parse_limit_and_session_selection(
        &["--limit".into(), "5".into()],
        Some("active".into()),
        20,
    )
    .unwrap();
    assert_eq!(limit, 5);
    assert_eq!(id, "active");
    assert!(!explicit);

    let (limit, id, explicit) =
        parse_limit_and_session_selection(&["7".into(), "session-id".into()], None, 20).unwrap();
    assert_eq!(limit, 7);
    assert_eq!(id, "session-id");
    assert!(explicit);

    let (_limit, id, explicit) =
        parse_limit_and_session_selection(&["--current".into()], Some("active".into()), 20)
            .unwrap();
    assert_eq!(id, "active");
    assert!(explicit);

    let dir = tempdir().unwrap();
    let (_id, path, explicit) = parse_export_args(
        dir.path(),
        Some("active".into()),
        &[".deepcli/exports/out.json".into()],
    )
    .unwrap();
    assert!(!explicit);
    assert_eq!(path.unwrap(), dir.path().join(".deepcli/exports/out.json"));
    let (_id, _path, explicit) =
        parse_export_args(dir.path(), Some("active".into()), &["--current".into()]).unwrap();
    assert!(explicit);
    assert!(parse_export_args(dir.path(), Some("active".into()), &["../out.json".into()]).is_err());

    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let prefix = session.id().to_string()[..8].to_string();
    let (id, path, explicit) = parse_export_args(dir.path(), None, &[prefix]).unwrap();
    let full_id = session.id().to_string();
    assert_eq!(id.as_deref(), Some(full_id.as_str()));
    assert!(path.is_none());
    assert!(explicit);
}

#[test]
fn parse_verify_args_accepts_repeated_path_scope() {
    let options = parse_verify_args(
        &[
            "--path".into(),
            "./src/commands.rs".into(),
            "--path=docs/ai".into(),
            "--limit".into(),
            "3".into(),
        ],
        Some("active".into()),
    )
    .unwrap();

    assert_eq!(
        options.path_filters,
        vec!["src/commands.rs".to_string(), "docs/ai".to_string()]
    );
    assert_eq!(options.limit, 3);
    assert_eq!(options.session_id.as_deref(), Some("active"));
    assert!(!options.fail_on_blockers);
    assert_eq!(options.output_path, None);
    assert!(parse_verify_args(&["--path".into(), "../secret".into()], None).is_err());

    let strict = parse_verify_args(&["--fail-on-blockers".into()], None).unwrap();
    assert!(strict.fail_on_blockers);

    let json = parse_verify_args(&["--json".into()], None).unwrap();
    assert!(json.json_output);

    let env = parse_verify_args(
        &[
            "--env-check".into(),
            "compiler".into(),
            "--env=docker".into(),
            "--env-check".into(),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        env.env_checks,
        vec!["compiler".to_string(), "docker".to_string(),]
    );
    assert!(parse_verify_args(&["--env-check".into(), "auto".into()], None).is_err());

    let output = parse_verify_args(
        &["--output".into(), ".deepcli/exports/verify.json".into()],
        None,
    )
    .unwrap();
    assert_eq!(
        output.output_path.as_deref(),
        Some(".deepcli/exports/verify.json")
    );
    assert!(parse_verify_args(
        &["--output".into(), "a.json".into(), "--output=b.json".into()],
        None
    )
    .is_err());
}

#[test]
fn parse_diff_and_review_args_accept_path_scope() {
    let diff = parse_diff_args(&[
        "--staged".into(),
        "--stat".into(),
        "--limit".into(),
        "10".into(),
        "--path".into(),
        "./src".into(),
        "--path=docs/ai".into(),
    ])
    .unwrap();
    assert!(diff.staged);
    assert_eq!(diff.view, DiffView::Stat);
    assert_eq!(diff.limit, Some(10));
    assert_eq!(
        diff.path_filters,
        vec!["src".to_string(), "docs/ai".to_string()]
    );
    assert!(parse_diff_args(&["--stat".into(), "--name-only".into()]).is_err());

    let review = parse_review_args(&["--scope".into(), "src/commands.rs".into()]).unwrap();
    assert_eq!(review, vec!["src/commands.rs".to_string()]);
    assert!(parse_review_args(&["--staged".into()]).is_err());
}

#[test]
fn parse_handoff_args_accepts_scope_limit_and_session() {
    let options = parse_handoff_args(
        &[
            "--path".into(),
            "./src".into(),
            "--limit=3".into(),
            "abc123".into(),
        ],
        None,
    )
    .unwrap();

    assert_eq!(options.path_filters, vec!["src".to_string()]);
    assert_eq!(options.limit, 3);
    assert_eq!(options.session_id.as_deref(), Some("abc123"));
    assert!(options.explicit_session);
    assert_eq!(options.format, HandoffFormat::Text);
    assert!(!options.fail_on_blockers);
    assert_eq!(options.output_path, None);

    let markdown = parse_handoff_args(&["--markdown".into()], None).unwrap();
    assert_eq!(markdown.format, HandoffFormat::Markdown);

    let pr = parse_handoff_args(&["--pr".into()], None).unwrap();
    assert_eq!(pr.format, HandoffFormat::PullRequest);

    let pr_format = parse_handoff_args(&["--format=pull-request".into()], None).unwrap();
    assert_eq!(pr_format.format, HandoffFormat::PullRequest);

    let json = parse_handoff_args(&["--format=json".into()], None).unwrap();
    assert_eq!(json.format, HandoffFormat::Json);

    let text = parse_handoff_args(&["--format".into(), "plain".into()], None).unwrap();
    assert_eq!(text.format, HandoffFormat::Text);

    let strict = parse_handoff_args(&["--fail-on-blockers".into()], None).unwrap();
    assert!(strict.fail_on_blockers);

    let output =
        parse_handoff_args(&["--output".into(), ".deepcli/exports/pr.md".into()], None).unwrap();
    assert_eq!(
        output.output_path.as_deref(),
        Some(".deepcli/exports/pr.md")
    );

    let env = parse_handoff_args(
        &[
            "--env-check".into(),
            "compiler".into(),
            "--env=docker".into(),
            "--env-check".into(),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        env.env_checks,
        vec!["compiler".to_string(), "docker".to_string(),]
    );
    assert!(parse_handoff_args(&["--env-check".into(), "auto".into()], None).is_err());

    assert!(parse_handoff_args(&["--path".into(), "../secret".into()], None).is_err());
    assert!(parse_handoff_args(&["--json".into(), "--markdown".into()], None).is_err());
    assert!(parse_handoff_args(&["--pr".into(), "--json".into()], None).is_err());
    assert!(parse_handoff_args(
        &["--output".into(), "a.md".into(), "--output=b.md".into()],
        None
    )
    .is_err());
}

#[test]
fn diff_stat_and_name_only_summarize_files_with_limits() {
    let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
-old
+new
+extra
diff --git a/docs/b.md b/docs/b.md
--- a/docs/b.md
+++ b/docs/b.md
+doc
";

    let stat = format_diff_stat(diff, Some(1));
    assert!(stat.contains("diff stat: 2 file(s), +3 -1"));
    assert!(stat.contains("- src/a.rs +2 -1"));
    assert!(stat.contains("... 1 more file(s)"));
    assert!(!stat.contains("docs/b.md +1 -0"));

    let names = format_diff_name_only(diff, None);
    assert!(names.contains("diff files: 2 file(s)"));
    assert!(names.contains("- src/a.rs"));
    assert!(names.contains("- docs/b.md"));
}

#[test]
fn weak_test_command_detection_flags_smoke_only_commands() {
    assert!(weak_test_command_reason("printf ok").is_some());
    assert!(weak_test_command_reason("echo ok").is_some());
    assert!(weak_test_command_reason("true").is_some());
    assert!(weak_test_command_reason("cargo test --quiet").is_none());
}

#[test]
fn summarizes_provider_usage_from_audit_events() {
    let id = uuid::Uuid::new_v4();
    let events = vec![
        AuditEvent {
            session_id: id,
            event_type: "provider_turn_started".to_string(),
            payload: json!({"request": {"total_bytes": 4096, "compacted": true}}),
            created_at: chrono::Utc::now(),
        },
        AuditEvent {
            session_id: id,
            event_type: "provider_turn_completed".to_string(),
            payload: json!({
                "elapsed_ms": 2500,
                "tool_calls": 2,
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 2,
                    "total_tokens": 12,
                    "prompt_cache_hit_tokens": 8,
                    "prompt_cache_miss_tokens": 2
                }
            }),
            created_at: chrono::Utc::now(),
        },
    ];

    let summary = summarize_audit_usage(&events);
    assert_eq!(summary.provider_turns_started, 1);
    assert_eq!(summary.provider_turns_completed, 1);
    assert_eq!(summary.provider_elapsed_ms, 2500);
    assert_eq!(summary.provider_max_elapsed_ms, Some(2500));
    assert_eq!(summary.provider_tool_calls, 2);
    assert_eq!(summary.compacted_turns, 1);
    assert_eq!(summary.prompt_tokens, Some(10));
    assert_eq!(summary.total_tokens, Some(12));
    assert_eq!(summary.max_request_bytes, Some(4096));
}

#[test]
fn usage_diagnostics_surface_latency_probe_and_failure_signals() {
    let id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    let events = vec![
        AuditEvent {
            session_id: id,
            event_type: "provider_turn_started".to_string(),
            payload: json!({
                "request": {
                    "total_bytes": 700_000,
                    "compacted": true
                }
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "provider_turn_completed".to_string(),
            payload: json!({
                "elapsed_ms": 35_000,
                "tool_calls": 3,
                "usage": {
                    "prompt_cache_hit_tokens": 1,
                    "prompt_cache_miss_tokens": 9
                }
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "provider_probe".to_string(),
            payload: json!({
                "provider": "deepseek",
                "status": "failed",
                "elapsed_ms": 120,
                "message": "401 unauthorized"
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "tool_failed".to_string(),
            payload: json!({"tool": "run_shell", "error": "boom"}),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "test_run".to_string(),
            payload: json!({"passed": false, "command": "cargo test"}),
            created_at: now,
        },
    ];

    let summary = summarize_audit_usage(&events);
    let diagnostics = format_usage_diagnostics(&summary, &events);
    assert!(diagnostics.contains("diagnostics:"));
    assert!(diagnostics.contains("slow provider responses detected"));
    assert!(diagnostics.contains("large provider requests"));
    assert!(diagnostics.contains("context compaction happened"));
    assert!(diagnostics.contains("provider probes: ok=0 skipped=0 failed=1 timeout=0"));
    assert!(diagnostics.contains("tool failures recorded: 1"));
    assert!(diagnostics.contains("failed test runs recorded: 1"));
}

#[test]
fn formats_audit_trace_for_slow_response_debugging() {
    let id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    let events = vec![
        AuditEvent {
            session_id: id,
            event_type: "provider_turn_started".to_string(),
            payload: json!({
                "iteration": 1,
                "timeout_seconds": 600,
                "request": {
                    "message_count": 4,
                    "tool_count": 21,
                    "total_bytes": 4096,
                    "compacted": false
                }
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "provider_turn_completed".to_string(),
            payload: json!({
                "elapsed_ms": 2500,
                "tool_calls": 2,
                "usage": {"total_tokens": 128}
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "provider_probe".to_string(),
            payload: json!({
                "provider": "deepseek",
                "status": "skipped",
                "elapsed_ms": 1,
                "message": "api_key missing"
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "tool_call".to_string(),
            payload: json!({
                "tool": "read_file",
                "status": "succeeded",
                "decision": {"risk": "low", "outcome": "allowed"}
            }),
            created_at: now,
        },
        AuditEvent {
            session_id: id,
            event_type: "credentials_updated".to_string(),
            payload: json!({
                "provider": "deepseek",
                "source": "hidden_prompt"
            }),
            created_at: now,
        },
    ];

    let trace = format_audit_trace(&events, 10);
    assert!(trace.contains("provider_turn_started"));
    assert!(trace.contains("request=4096 bytes"));
    assert!(trace.contains("elapsed=2500ms"));
    assert!(trace.contains("provider_probe provider=deepseek status=skipped"));
    assert!(trace.contains("tool_call tool=read_file"));
    assert!(trace.contains("credentials_updated provider=deepseek source=hidden_prompt"));
    assert!(trace.contains("apiKey=<redacted>"));
}

#[test]
fn trace_falls_back_to_latest_session_with_audit_events() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let with_audit = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    with_audit
        .append_audit_event(
            "credentials_updated",
            json!({"provider": "deepseek", "source": "set"}),
        )
        .unwrap();
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();

    let output =
        handle_trace(dir.path(), Some(current_empty.id().to_string()), Vec::new()).unwrap();
    assert!(output.contains("latest session with audit events"));
    assert!(output.contains(&with_audit.id().to_string()));
    assert!(output.contains("credentials_updated provider=deepseek source=set"));

    let no_current = handle_trace(dir.path(), None, Vec::new()).unwrap();
    assert!(no_current.contains("latest session with audit events; no current session"));
    assert!(no_current.contains(&with_audit.id().to_string()));

    let explicit = handle_trace(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![current_empty.id().to_string()],
    )
    .unwrap();
    assert!(explicit.contains("no audit events"));
    assert!(!explicit.contains("latest session with audit events"));
}

#[test]
fn trace_json_output_is_structured_redacted_and_written() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("trace export").unwrap();
    session
        .append_audit_event(
            "provider_turn_started",
            json!({
                "request": {
                    "total_bytes": 4096,
                    "compacted": false
                }
            }),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_probe",
            json!({
                "provider": "deepseek",
                "status": "failed",
                "elapsed_ms": 12,
                "message": "api_key: secret-value",
                "apiKey": "secret-value"
            }),
        )
        .unwrap();

    let output = handle_trace(
        dir.path(),
        None,
        vec![
            "--json".into(),
            "--limit".into(),
            "1".into(),
            "--output".into(),
            ".deepcli/exports/trace.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.trace.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["sessionSource"], "latest");
    assert_eq!(value["session"]["title"], "trace export");
    assert_eq!(value["limit"], 1);
    assert_eq!(value["totalEvents"], 2);
    assert_eq!(value["shownEvents"], 1);
    assert_eq!(value["events"][0]["eventType"], "provider_probe");
    assert_eq!(value["events"][0]["payload"]["apiKey"], "<redacted>");
    assert!(value["events"][0]["payload"]["message"]
        .as_str()
        .unwrap()
        .contains("<redacted>"));
    assert!(value["events"][0]["line"]
        .as_str()
        .unwrap()
        .contains("provider_probe provider=deepseek status=failed"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("showing latest 1/2"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/trace.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn trace_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();

    let error = handle_trace(
        dir.path(),
        None,
        vec!["--output".into(), "../trace.txt".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../trace.txt").exists());
}

#[test]
fn logs_json_output_tails_latest_log_redacts_and_writes() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
    fs::write(
        dir.path().join(".deepcli/logs/deepcli.log"),
        "first\napi_key = sk-log-secret\nlast\n",
    )
    .unwrap();

    let output = handle_logs(
        dir.path(),
        vec![
            "--json".into(),
            "--limit".into(),
            "2".into(),
            "--output".into(),
            ".deepcli/exports/logs.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.logs.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["logsDir"], ".deepcli/logs");
    assert_eq!(value["limit"], 2);
    assert_eq!(value["fileCount"], 1);
    assert_eq!(value["selectedFile"]["name"], "deepcli.log");
    assert_eq!(value["lineCount"], 2);
    assert_eq!(value["totalLines"], 3);
    assert_eq!(value["truncated"], true);
    assert!(value["lines"][0].as_str().unwrap().contains("<redacted>"));
    assert_eq!(value["lines"][1], "last");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli support"));
    assert!(!output.contains("sk-log-secret"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/logs.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn logs_list_and_empty_output_are_structured() {
    let dir = tempdir().unwrap();
    let empty = handle_logs(dir.path(), vec!["--json".into()]).unwrap();
    let empty_value: Value = serde_json::from_str(&empty).unwrap();
    assert_eq!(empty_value["schema"], "deepcli.logs.v1");
    assert_eq!(empty_value["status"], "no_logs");
    let empty_next_actions = json_string_array(&empty_value["nextActions"]);
    assert_executable_deepcli_actions(&empty_next_actions);
    assert_checklist_matches_executable_actions(&empty_value, &empty_next_actions);
    assert_eq!(
        empty_next_actions[0],
        "deepcli diagnose --bundle .deepcli/support/latest"
    );

    fs::create_dir_all(dir.path().join(".deepcli/logs")).unwrap();
    fs::write(dir.path().join(".deepcli/logs/first.log"), "one\n").unwrap();
    let list = handle_logs(dir.path(), vec!["--list".into()]).unwrap();
    assert!(list.contains("first.log"));
    assert!(list.contains("tail: skipped because --list was requested"));
}

#[test]
fn logs_reject_unsafe_paths() {
    let dir = tempdir().unwrap();

    let file_error = handle_logs(dir.path(), vec!["--file".into(), "../secret.log".into()])
        .unwrap_err()
        .to_string();
    assert!(file_error.contains("log file path traversal is not allowed"));

    let output_error = handle_logs(dir.path(), vec!["--output".into(), "../logs.json".into()])
        .unwrap_err()
        .to_string();
    assert!(output_error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../logs.json").exists());
}

#[test]
fn status_falls_back_to_latest_session_and_shows_usage_context() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let executor = test_executor(dir.path());
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("status focus").unwrap();
    session.append_message("user", "check status").unwrap();
    session
        .append_audit_event(
            "provider_turn_started",
            json!({
                "request": {
                    "total_bytes": 4096,
                    "compacted": true
                }
            }),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            json!({
                "elapsed_ms": 321,
                "tool_calls": 2,
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 7,
                    "total_tokens": 18,
                    "prompt_cache_hit_tokens": 5,
                    "prompt_cache_miss_tokens": 3
                }
            }),
        )
        .unwrap();

    let output = handle_status(
        CommandContext {
            workspace: dir.path(),
            config: &config,
            registry: &registry,
            executor: &executor,
            session_id: None,
            provider_override: None,
            allow_interactive_prompts: true,
        },
        Vec::new(),
    )
    .unwrap();

    assert!(output.contains("session: <none>"));
    assert!(output.contains("latest session:"));
    assert!(output.contains("status focus"));
    assert!(output.contains("provider turns: started=1 completed=1 total_elapsed_ms=321"));
    assert!(output.contains("tokens: prompt=11 completion=7 total=18 cache_hit=5 cache_miss=3"));
    assert!(output.contains("context: compacted_turns=1 audit_events=2 max_request_bytes=4096 latest_request_bytes=4096"));
    assert!(output.contains("note: no active session; showing latest recorded activity"));
    assert!(output.contains("/resume"));
}

#[test]
fn status_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let executor = test_executor(dir.path());
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("status json").unwrap();
    session.append_message("user", "show status").unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            json!({
                "elapsed_ms": 123,
                "tool_calls": 1,
                "usage": {"total_tokens": 9}
            }),
        )
        .unwrap();

    let output = handle_status(
        CommandContext {
            workspace: dir.path(),
            config: &config,
            registry: &registry,
            executor: &executor,
            session_id: None,
            provider_override: None,
            allow_interactive_prompts: true,
        },
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/status.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.status.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["sessionSource"], "latest");
    assert_eq!(value["session"]["title"], "status json");
    assert_eq!(value["session"]["activity"]["messages"], 1);
    assert_eq!(value["session"]["usage"]["totalTokens"], 9);
    let short = value["session"]["shortId"].as_str().unwrap();
    let next_actions = json_string_array(&value["session"]["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert_checklist_matches_executable_actions(&value["session"], &next_actions);
    assert_eq!(
        json_checklist_labels(&value),
        vec!["Inspect session usage", "Inspect session trace"]
    );
    assert_eq!(
        next_actions,
        vec![
            format!("deepcli usage {short}"),
            format!("deepcli trace --limit 20 {short}")
        ]
    );
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("latest session:"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/status.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn status_json_session_actions_are_executable_for_next_action_signals() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let executor = test_executor(dir.path());
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.append_message("user", "needs next action").unwrap();
    session.set_state(SessionState::WaitingUser).unwrap();

    let output = handle_status(
        CommandContext {
            workspace: dir.path(),
            config: &config,
            registry: &registry,
            executor: &executor,
            session_id: None,
            provider_override: None,
            allow_interactive_prompts: true,
        },
        vec!["--json".into()],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.status.v1");
    assert_eq!(value["session"]["nextActionSignals"], true);
    let short = value["session"]["shortId"].as_str().unwrap();
    let next_actions = json_string_array(&value["session"]["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert_checklist_matches_executable_actions(&value["session"], &next_actions);
    assert_eq!(
        json_checklist_labels(&value),
        vec!["Inspect recovery actions", "Inspect session diagnostics"]
    );
    assert_eq!(
        next_actions,
        vec![
            format!("deepcli next {short}"),
            format!("deepcli session diagnose {short}")
        ]
    );
}

#[test]
fn status_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let registry = ToolRegistry::mvp();
    let executor = test_executor(dir.path());

    let error = handle_status(
        CommandContext {
            workspace: dir.path(),
            config: &config,
            registry: &registry,
            executor: &executor,
            session_id: None,
            provider_override: None,
            allow_interactive_prompts: true,
        },
        vec!["--output".into(), "../status.txt".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../status.txt").exists());
}

#[test]
fn usage_supports_explicit_session_and_falls_back_from_empty_current() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let with_usage = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    with_usage
        .append_audit_event(
            "provider_turn_completed",
            json!({
                "elapsed_ms": 123,
                "tool_calls": 0,
                "usage": {"total_tokens": 9}
            }),
        )
        .unwrap();
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();

    let fallback =
        handle_usage(dir.path(), Some(current_empty.id().to_string()), Vec::new()).unwrap();
    assert!(fallback.contains("latest session with recorded usage/activity"));
    assert!(fallback.contains(&with_usage.id().to_string()));
    assert!(fallback.contains("audit_events: 1"));
    assert!(fallback.contains("latest=provider_turn_completed"));
    assert!(fallback.contains("total=9"));

    let explicit = handle_usage(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![current_empty.id().to_string()],
    )
    .unwrap();
    assert!(explicit.contains(&current_empty.id().to_string()));
    assert!(explicit.contains("messages=0"));
    assert!(explicit.contains("no provider turns recorded for this session"));
    assert!(!explicit.contains("latest session with recorded usage/activity"));

    let current = handle_usage(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["--current".to_string()],
    )
    .unwrap();
    assert!(current.contains(&current_empty.id().to_string()));
    assert!(!current.contains("latest session with recorded usage/activity"));

    let no_current = handle_usage(dir.path(), None, Vec::new()).unwrap();
    assert!(no_current.contains("latest session with recorded usage/activity; no current session"));
    assert!(no_current.contains(&with_usage.id().to_string()));
}

#[test]
fn usage_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("slow response").unwrap();
    session.append_message("user", "why slow").unwrap();
    session.write_summary("latency investigation").unwrap();
    session
        .append_audit_event(
            "provider_turn_started",
            json!({
                "request": {
                    "total_bytes": 700000,
                    "compacted": true
                }
            }),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            json!({
                "elapsed_ms": 45000,
                "tool_calls": 2,
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 20,
                    "total_tokens": 120,
                    "prompt_cache_hit_tokens": 10,
                    "prompt_cache_miss_tokens": 90
                }
            }),
        )
        .unwrap();
    session
        .append_audit_event(
            "provider_probe",
            json!({
                "provider": "deepseek",
                "status": "failed",
                "elapsed_ms": 100,
                "message": "401 unauthorized"
            }),
        )
        .unwrap();
    session
        .append_audit_event("tool_failed", json!({"tool": "run_shell", "error": "boom"}))
        .unwrap();
    session
        .append_audit_event(
            "test_run",
            json!({"passed": false, "command": "cargo test"}),
        )
        .unwrap();

    let output = handle_usage(
        dir.path(),
        None,
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/usage.json".into(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.usage.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["sessionSource"], "latest");
    assert_eq!(value["session"]["title"], "slow response");
    assert_eq!(value["session"]["activity"]["messages"], 1);
    assert_eq!(value["session"]["providerTurns"]["completed"], 1);
    assert_eq!(value["session"]["providerTurns"]["averageElapsedMs"], 45000);
    assert_eq!(value["session"]["tokens"]["total"], 120);
    assert_eq!(value["session"]["request"]["maxBytes"], 700000);
    assert_eq!(value["session"]["context"]["compactedTurns"], 1);
    assert_eq!(value["session"]["failedTools"], 1);
    assert_eq!(value["session"]["failedTests"], 1);
    assert_eq!(value["session"]["summaryPreview"], "latency investigation");
    let short = value["session"]["shortId"].as_str().unwrap();
    let next_actions = json_string_array(&value["session"]["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert_checklist_matches_executable_actions(&value["session"], &next_actions);
    assert_eq!(
        json_checklist_labels(&value),
        vec!["Inspect session trace", "Inspect session diagnostics"]
    );
    assert_eq!(
        next_actions,
        vec![
            format!("deepcli trace --limit 20 {short}"),
            format!("deepcli session diagnose {short}")
        ]
    );
    let diagnostics = value["session"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|item| {
        item.as_str()
            .unwrap()
            .contains("slow provider responses detected")
    }));
    assert!(diagnostics.iter().any(|item| {
        item.as_str()
            .unwrap()
            .contains("provider probes: ok=0 skipped=0 failed=1 timeout=0")
    }));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("largest provider request"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/usage.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn usage_text_output_does_not_echo_session_summary_body() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session
        .write_summary("上一轮 assistant 的长回复内容不应出现在普通 usage 文本里")
        .unwrap();
    session
        .append_audit_event(
            "provider_turn_completed",
            json!({
                "elapsed_ms": 123,
                "tool_calls": 0,
                "usage": {"total_tokens": 9}
            }),
        )
        .unwrap();

    let output = handle_usage(dir.path(), Some(session.id().to_string()), Vec::new()).unwrap();

    assert!(output.contains("activity:"));
    assert!(output.contains("summary=true"));
    assert!(!output.contains("summary preview:"));
    assert!(!output.contains("上一轮 assistant 的长回复内容"));
}

#[test]
fn usage_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();

    let error = handle_usage(
        dir.path(),
        None,
        vec!["--output".into(), "../usage.txt".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../usage.txt").exists());
}

#[test]
fn session_inspection_commands_fall_back_by_content_type() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let populated = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    populated
        .append_message("user", "inspect this session")
        .unwrap();
    populated.write_summary("saved session summary").unwrap();
    populated
        .append_tool_call(&ToolCallRecord {
            tool: "read_file".to_string(),
            input: json!({"path": "Cargo.toml"}),
            output: json!({"apiKey": "sk-session-secret", "ok": true}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    populated
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    populated
        .save_diff(
            "src/lib.rs",
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n+diffed\n+api_key = sk-session-secret\n",
        )
        .unwrap();
    populated
        .save_backup(
            "src/lib.rs",
            "original backup content\napi_key = sk-session-secret\n",
        )
        .unwrap();
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let current_id = Some(current_empty.id().to_string());

    let history = handle_session(dir.path(), current_id.clone(), vec!["history".into()]).unwrap();
    assert!(history.contains("latest session with messages"));
    assert!(history.contains("inspect this session"));

    let summary = handle_session(dir.path(), current_id.clone(), vec!["summary".into()]).unwrap();
    assert!(summary.contains("latest session with a saved summary"));
    assert!(summary.contains("saved session summary"));

    let tools = handle_session(dir.path(), current_id.clone(), vec!["tools".into()]).unwrap();
    assert!(tools.contains("latest session with tool calls"));
    assert!(tools.contains("read_file"));

    let tests = handle_session(dir.path(), current_id.clone(), vec!["tests".into()]).unwrap();
    assert!(tests.contains("latest session with test runs"));
    assert!(tests.contains("cargo test"));

    let diffs = handle_session(dir.path(), current_id.clone(), vec!["diffs".into()]).unwrap();
    assert!(diffs.contains("latest session with diff records"));
    assert!(diffs.contains("+diffed"));

    let backups = handle_session(dir.path(), current_id.clone(), vec!["backups".into()]).unwrap();
    assert!(backups.contains("latest session with backup records"));
    assert!(backups.contains("target=src/lib.rs"));
    assert!(backups.contains("original backup content"));

    let show = handle_session(dir.path(), current_id.clone(), vec!["show".into()]).unwrap();
    assert!(show.contains("latest session with recorded activity"));
    assert!(show.contains(&populated.id().to_string()));

    let export = handle_session(dir.path(), current_id.clone(), vec!["export".into()]).unwrap();
    assert!(export.contains("latest session with recorded activity"));
    assert!(export.contains(&populated.id().to_string()));
    assert!(dir
        .path()
        .join(format!(".deepcli/exports/session-{}.json", populated.id()))
        .exists());
    let export_path = dir
        .path()
        .join(format!(".deepcli/exports/session-{}.json", populated.id()));
    let exported: Value = serde_json::from_str(&fs::read_to_string(export_path).unwrap()).unwrap();
    assert!(exported["diffs"][0]["content"]
        .as_str()
        .unwrap()
        .contains("+diffed"));
    assert!(exported["backups"][0]["content"]
        .as_str()
        .unwrap()
        .contains("original backup content"));

    let history_without_current = handle_session(dir.path(), None, vec!["history".into()]).unwrap();
    assert!(history_without_current.contains("latest session with messages; no current session"));
    assert!(history_without_current.contains("inspect this session"));

    let summary_without_current = handle_session(dir.path(), None, vec!["summary".into()]).unwrap();
    assert!(
        summary_without_current.contains("latest session with a saved summary; no current session")
    );
    assert!(summary_without_current.contains("saved session summary"));

    let tools_without_current = handle_session(dir.path(), None, vec!["tools".into()]).unwrap();
    assert!(tools_without_current.contains("latest session with tool calls; no current session"));
    assert!(tools_without_current.contains("read_file"));

    let tests_without_current = handle_session(dir.path(), None, vec!["tests".into()]).unwrap();
    assert!(tests_without_current.contains("latest session with test runs; no current session"));
    assert!(tests_without_current.contains("cargo test"));

    let diffs_without_current = handle_session(dir.path(), None, vec!["diffs".into()]).unwrap();
    assert!(diffs_without_current.contains("latest session with diff records; no current session"));
    assert!(diffs_without_current.contains("+diffed"));

    let backups_without_current = handle_session(dir.path(), None, vec!["backups".into()]).unwrap();
    assert!(
        backups_without_current.contains("latest session with backup records; no current session")
    );
    assert!(backups_without_current.contains("original backup content"));

    let show_without_current = handle_session(dir.path(), None, vec!["show".into()]).unwrap();
    assert!(
        show_without_current.contains("latest session with recorded activity; no current session")
    );
    assert!(show_without_current.contains(&populated.id().to_string()));

    let export_without_current = handle_session(dir.path(), None, vec!["export".into()]).unwrap();
    assert!(export_without_current
        .contains("latest session with recorded activity; no current session"));
    assert!(export_without_current.contains(&populated.id().to_string()));

    let current_history = handle_session(
        dir.path(),
        current_id,
        vec!["history".into(), "--current".into()],
    )
    .unwrap();
    assert!(current_history.contains("no messages"));
    assert!(!current_history.contains("latest session with messages"));

    let history_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![
            "history".into(),
            "--limit".into(),
            "5".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/session-history.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&history_json).unwrap();
    assert_eq!(value["schema"], "deepcli.session.inspect.v1");
    assert_eq!(value["kind"], "history");
    assert_eq!(value["session"]["id"], populated.id().to_string());
    assert_eq!(value["activity"]["messages"], 1);
    assert_eq!(value["payload"]["recordCount"], 1);
    assert_eq!(value["payload"]["messages"][0]["role"], "user");
    assert!(value["payload"]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("inspect this session"));
    assert!(value["note"]
        .as_str()
        .unwrap()
        .contains("latest session with messages"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action
            == &format!("deepcli session next {} --json", short_id(&populated.id()))));
    assert!(next_actions.iter().any(|action| action
        == &format!(
            "deepcli session diagnose {} --json",
            short_id(&populated.id())
        )));
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
    assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
    assert!(checklist_labels.contains(&"List saved sessions".to_string()));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/session-history.json")).unwrap();
    assert_eq!(written, history_json);

    let summary_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["summary".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&summary_json).unwrap();
    assert_eq!(value["kind"], "summary");
    assert_eq!(value["payload"]["summary"], "saved session summary");

    let tools_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["tools".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&tools_json).unwrap();
    assert_eq!(value["kind"], "tools");
    assert_eq!(value["payload"]["tools"][0]["tool"], "read_file");
    assert_eq!(
        value["payload"]["tools"][0]["output"]["apiKey"],
        "<redacted>"
    );
    assert!(!tools_json.contains("sk-session-secret"));

    let tests_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["tests".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&tests_json).unwrap();
    assert_eq!(value["kind"], "tests");
    assert_eq!(value["payload"]["tests"][0]["command"], "cargo test");

    let diffs_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["diffs".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&diffs_json).unwrap();
    assert_eq!(value["kind"], "diffs");
    assert!(value["payload"]["diffs"][0]["content"]
        .as_str()
        .unwrap()
        .contains("+diffed"));
    assert!(!diffs_json.contains("sk-session-secret"));

    let backups_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["backups".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&backups_json).unwrap();
    assert_eq!(value["kind"], "backups");
    assert_eq!(value["payload"]["backups"][0]["targetPath"], "src/lib.rs");
    assert!(value["payload"]["backups"][0]["content"]
        .as_str()
        .unwrap()
        .contains("original backup content"));
    assert!(!backups_json.contains("sk-session-secret"));

    let show_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["show".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&show_json).unwrap();
    assert_eq!(value["kind"], "show");
    assert_eq!(value["payload"]["activity"]["messages"], 1);
}

#[test]
fn session_tools_failed_filter_jumps_to_failed_tool_calls() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let failed = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    failed
        .append_tool_call(&ToolCallRecord {
            tool: "read_file".to_string(),
            input: json!({"path": "Cargo.toml"}),
            output: json!({"ok": true}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    failed
        .append_tool_call(&ToolCallRecord {
            tool: "run_shell".to_string(),
            input: json!({"command": "cargo test"}),
            output: json!({"error": "tests failed"}),
            decision: None,
            status: ToolCallStatus::Failed,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(2));
    let newer_success = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    newer_success
        .append_tool_call(&ToolCallRecord {
            tool: "list_files".to_string(),
            input: json!({}),
            output: json!({"files": ["Cargo.toml"]}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let output = handle_session(
        dir.path(),
        None,
        vec![
            "tools".into(),
            "--failed".into(),
            "--limit".into(),
            "5".into(),
        ],
    )
    .unwrap();
    assert!(output.contains("latest session with failed tool calls; no current session"));
    assert!(output.contains(&failed.id().to_string()));
    assert!(output.contains("showing latest 1 failed or denied tool call"));
    assert!(output.contains("tool=run_shell"));
    assert!(output.contains("tests failed"));
    assert!(!output.contains("tool=list_files"));
    assert!(output.contains("next: inspect `/trace --limit 30`"));

    let explicit_success = handle_session(
        dir.path(),
        None,
        vec![
            "tools".into(),
            "--failed".into(),
            newer_success.id().to_string(),
        ],
    )
    .unwrap();
    assert!(explicit_success.contains("no failed or denied tool calls"));
}

#[test]
fn session_next_actions_aggregate_recovery_signals() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut actionable = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    actionable.rename("compiler repair").unwrap();
    actionable
        .set_state(SessionState::AwaitingApproval)
        .unwrap();
    actionable
        .enqueue_approval_request(
            "write_file",
            crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    actionable
        .enqueue_side_question("should we switch to v4-flash?")
        .unwrap();
    actionable
        .append_tool_call(&ToolCallRecord {
            tool: "run_shell".to_string(),
            input: json!({"command": "cargo test"}),
            output: json!({"apiKey": "sk-secret-value", "error": "tests failed"}),
            decision: None,
            status: ToolCallStatus::Failed,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    actionable
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(101),
            stdout: String::new(),
            stderr: "test failed".to_string(),
            passed: false,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    actionable
        .save_plan(&Plan {
            title: "repair compiler".to_string(),
            steps: vec![
                PlanStep {
                    id: "1".to_string(),
                    description: "fix parser regression".to_string(),
                    status: PlanStepStatus::Failed,
                },
                PlanStep {
                    id: "2".to_string(),
                    description: "rerun compiler tests".to_string(),
                    status: PlanStepStatus::Pending,
                },
            ],
            updated_at: chrono::Utc::now(),
        })
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(2));
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();

    let output = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["next".into()],
    )
    .unwrap();
    assert!(output.contains("latest session with next action signals"));
    assert!(output.contains("compiler repair"));
    assert!(output.contains("next actions:"));
    assert!(output.contains("/approval list"));
    assert!(output.contains("/btw list"));
    assert!(output.contains("/session tools --failed --limit 5"));
    assert!(output.contains("latest tool=run_shell"));
    assert!(output.contains("/session tests --limit 5"));
    assert!(output.contains("latest command=cargo test"));
    assert!(output.contains("repair failed plan step `1`"));
    assert!(output.contains("/resume"));

    let json_output = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![
            "next".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/next.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(value["schema"], "deepcli.next.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["session"]["title"], "compiler repair");
    assert_eq!(value["signals"]["pendingApprovals"], 1);
    assert_eq!(value["signals"]["openByTheWayQuestions"], 1);
    assert_eq!(value["signals"]["failedOrDeniedTools"], 1);
    assert_eq!(value["signals"]["failedTests"], 1);
    assert_eq!(value["signals"]["incompletePlanSteps"], 2);
    assert!(value["signals"]["hasNextActionSignals"].as_bool().unwrap());
    let short = short_id(&actionable.id());
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(next_actions
        .iter()
        .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
    assert!(next_actions
        .iter()
        .all(|item| !item.as_str().unwrap().contains("`/")));
    assert!(next_actions
        .iter()
        .any(|item| { item.as_str() == Some(&format!("deepcli approval list {short} --json")) }));
    assert!(next_actions
        .iter()
        .any(|item| item.as_str() == Some(&format!("deepcli btw list {short} --json"))));
    assert!(next_actions.iter().any(|item| {
        item.as_str()
            == Some(&format!(
                "deepcli session tools --failed --limit 5 {short} --json"
            ))
    }));
    assert!(next_actions.iter().any(|item| {
        item.as_str() == Some(&format!("deepcli session tests --limit 5 {short} --json"))
    }));
    assert!(next_actions
        .iter()
        .any(|item| item.as_str() == Some(&format!("deepcli resume {short}"))));
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    let labels = json_checklist_labels(&value);
    assert!(labels.contains(&"Review approvals".to_string()));
    assert!(labels.contains(&"Review by-the-way questions".to_string()));
    assert!(labels.contains(&"Inspect failed tools".to_string()));
    assert!(labels.contains(&"Inspect session tests".to_string()));
    assert!(labels.contains(&"Resume saved work".to_string()));
    let quick_links = value["quickLinks"].as_array().unwrap();
    assert!(quick_links
        .iter()
        .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
    let quick_link_strings = json_string_array(&value["quickLinks"]);
    assert_checklist_matches_executable_actions(
        &json!({"checklist": value["quickLinkChecklist"].clone()}),
        &quick_link_strings,
    );
    let quick_link_labels = json!({"checklist": value["quickLinkChecklist"].clone()});
    let quick_link_labels = json_checklist_labels(&quick_link_labels);
    assert!(quick_link_labels.contains(&"Inspect session history".to_string()));
    assert!(quick_links
        .iter()
        .any(|item| item.as_str() == Some(&format!("deepcli resume {short}"))));
    assert!(value["report"].as_str().unwrap().contains("next actions:"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/next.json")).unwrap();
    assert_eq!(written, json_output);

    let diagnosis = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["diagnose".into(), "--limit".into(), "2".into()],
    )
    .unwrap();
    assert!(diagnosis.contains("latest session with next action signals"));
    assert!(diagnosis.contains("session diagnosis"));
    assert!(diagnosis.contains("signals:"));
    assert!(diagnosis.contains("pending approvals: 1"));
    assert!(diagnosis.contains("open by-the-way questions: 1"));
    assert!(diagnosis.contains("recent failed or denied tools: 1"));
    assert!(diagnosis.contains("failed test runs: 1"));
    assert!(diagnosis.contains("incomplete plan steps: 2"));
    assert!(diagnosis.contains("recent failures:"));
    assert!(diagnosis.contains("tool=run_shell"));
    assert!(diagnosis.contains("recent tests:"));
    assert!(diagnosis.contains("command=cargo test"));
    assert!(diagnosis.contains("plan status:"));
    assert!(diagnosis.contains("recommended next actions:"));
    assert!(diagnosis.contains("/session tools --failed --limit 2"));
    assert!(diagnosis.contains("<redacted>"));
    assert!(!diagnosis.contains("sk-secret-value"));

    let diagnosis_json = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![
            "diagnose".into(),
            "--limit".into(),
            "2".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/session-diagnose.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&diagnosis_json).unwrap();
    assert_eq!(value["schema"], "deepcli.session.diagnose.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["limit"], 2);
    assert_eq!(value["session"]["title"], "compiler repair");
    assert_eq!(value["signals"]["pendingApprovals"], 1);
    assert_eq!(value["signals"]["openByTheWayQuestions"], 1);
    assert_eq!(value["signals"]["failedOrDeniedTools"], 1);
    assert_eq!(value["signals"]["recentFailedOrDeniedTools"], 1);
    assert_eq!(value["signals"]["failedTests"], 1);
    assert_eq!(value["signals"]["incompletePlanSteps"], 2);
    assert_eq!(value["recentFailures"][0]["tool"], "run_shell");
    assert_eq!(value["recentFailures"][0]["output"]["apiKey"], "<redacted>");
    assert_eq!(value["recentTests"][0]["command"], "cargo test");
    assert_eq!(value["plan"]["incomplete"], 2);
    let recommended = value["recommendedNextActions"].as_array().unwrap();
    assert!(recommended
        .iter()
        .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
    assert!(recommended
        .iter()
        .any(|item| { item.as_str() == Some(&format!("deepcli approval list {short} --json")) }));
    let recommended_strings = json_string_array(&value["recommendedNextActions"]);
    assert_checklist_matches_executable_actions(&value, &recommended_strings);
    let labels = json_checklist_labels(&value);
    assert!(labels.contains(&"Review approvals".to_string()));
    assert!(labels.contains(&"Review by-the-way questions".to_string()));
    assert!(labels.contains(&"Inspect failed tools".to_string()));
    assert!(labels.contains(&"Inspect session tests".to_string()));
    assert!(labels.contains(&"Resume saved work".to_string()));
    let quick_links = value["quickLinks"].as_array().unwrap();
    assert!(quick_links
        .iter()
        .all(|item| item.as_str().unwrap().starts_with("deepcli ")));
    let quick_link_strings = json_string_array(&value["quickLinks"]);
    assert_checklist_matches_executable_actions(
        &json!({"checklist": value["quickLinkChecklist"].clone()}),
        &quick_link_strings,
    );
    let quick_link_labels = json!({"checklist": value["quickLinkChecklist"].clone()});
    let quick_link_labels = json_checklist_labels(&quick_link_labels);
    assert!(quick_link_labels.contains(&"Inspect session history".to_string()));
    assert!(quick_links
        .iter()
        .any(|item| item.as_str() == Some(&format!("deepcli usage {short} --json"))));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("session diagnosis"));
    assert!(!diagnosis_json.contains("sk-secret-value"));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/session-diagnose.json")).unwrap();
    assert_eq!(written, diagnosis_json);
}

#[test]
fn session_next_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();

    let error = handle_session(
        dir.path(),
        None,
        vec!["next".into(), "--output".into(), "../next.txt".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../next.txt").exists());

    let error = handle_session(
        dir.path(),
        None,
        vec![
            "diagnose".into(),
            "--output".into(),
            "../diagnose.txt".into(),
        ],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../diagnose.txt").exists());

    let error = handle_session(
        dir.path(),
        None,
        vec!["history".into(), "--output".into(), "../history.txt".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../history.txt").exists());
}

#[test]
fn session_next_actions_reports_clean_session_without_blockers() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let clean = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    clean.append_message("user", "done?").unwrap();
    clean
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let output = handle_session(dir.path(), None, vec!["next".into()]).unwrap();
    assert!(output.contains("latest session with recorded activity; no current session"));
    assert!(output.contains("no blocking signals found"));
    assert!(output.contains("/session history --limit 20"));
    assert!(output.contains("/usage"));
}

#[test]
fn session_search_finds_query_across_persisted_context() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut renamed = session.clone();
    renamed.rename("compiler repair").unwrap();
    session
        .append_message("user", "please fix parser panic api_key = sk-search-secret")
        .unwrap();
    session
        .write_summary("fixed compiler lv4 regression")
        .unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "read_file".to_string(),
            input: json!({"path": "src/parser.rs"}),
            output: json!({"content": "parser panic"}),
            decision: None,
            status: ToolCallStatus::Succeeded,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test parser".to_string(),
            exit_code: Some(101),
            stdout: String::new(),
            stderr: "parser failed".to_string(),
            passed: false,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    session.save_diff("src/parser.rs", "+parser fix\n").unwrap();
    session
        .save_backup("src/parser.rs", "parser before\n")
        .unwrap();

    let output = handle_session(
        dir.path(),
        None,
        vec![
            "search".into(),
            "parser".into(),
            "--limit".into(),
            "5".into(),
        ],
    )
    .unwrap();

    assert!(output.contains(&session.id().to_string()));
    assert!(output.contains(&format!("id={}", short_id(&session.id()))));
    assert!(output.contains(&format!("full={}", session.id())));
    assert!(output.contains("title=compiler repair"));
    assert!(output.contains("message/user"));
    assert!(output.contains("<redacted>"));
    assert!(!output.contains("sk-search-secret"));
    assert!(output.contains("tool: read_file"));
    assert!(output.contains("test: cargo test parser"));
    assert!(output.contains("diff:"));

    let json_output = handle_session(
        dir.path(),
        None,
        vec![
            "search".into(),
            "parser".into(),
            "--limit".into(),
            "5".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/session-search.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.search.v1");
    assert_eq!(value["query"], "parser");
    assert_eq!(value["limit"], 5);
    assert_eq!(value["hitCount"], 1);
    assert_eq!(value["hits"][0]["session"]["id"], session.id().to_string());
    assert_eq!(
        value["nextActions"][0],
        format!(
            "deepcli resume {} --dry-run --json",
            short_id(&session.id())
        )
    );
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Resume preview".to_string()));
    assert!(checklist_labels.contains(&"Inspect session history".to_string()));
    assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
    assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str()
            == Some(&format!(
                "deepcli session history {} --limit 20",
                short_id(&session.id())
            ))));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str()
            == Some(&format!(
                "deepcli session next {} --json",
                short_id(&session.id())
            ))));
    assert!(value["hits"][0]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("message/user")));
    assert!(!json_output.contains("sk-search-secret"));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/session-search.json")).unwrap();
    assert_eq!(written, json_output);
}

#[test]
fn session_search_reports_no_matches() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap()
        .append_message("user", "hello")
        .unwrap();

    let output = handle_session(dir.path(), None, vec!["search".into(), "missing".into()]).unwrap();
    assert_eq!(output, "no sessions matched `missing`");

    let json_output = handle_session(
        dir.path(),
        None,
        vec!["search".into(), "missing".into(), "--json".into()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.search.v1");
    assert_eq!(value["hitCount"], 0);
    assert_eq!(value["nextActions"][0], "deepcli sessions --all --limit 20");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"List saved sessions".to_string()));
    assert!(checklist_labels.contains(&"Resume preview".to_string()));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("deepcli resume --dry-run --json")));
}

#[test]
fn session_rename_updates_selected_history_without_switching() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let prefix = session.id().to_string()[..8].to_string();

    let output = handle_session(
        dir.path(),
        None,
        vec![
            "rename".into(),
            prefix,
            "compiler".into(),
            "lv9".into(),
            "repair".into(),
        ],
    )
    .unwrap();

    assert!(output.contains("renamed session"));
    assert!(output.contains("id="));
    assert!(output.contains("title=compiler lv9 repair"));
    let loaded = store.load(&session.id().to_string()).unwrap();
    assert_eq!(
        loaded.metadata.title.as_deref(),
        Some("compiler lv9 repair")
    );
}

#[test]
fn session_rename_current_requires_active_session() {
    let dir = tempdir().unwrap();
    let error = handle_session(
        dir.path(),
        None,
        vec!["rename".into(), "--current".into(), "new".into()],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("no active session"));
}

#[test]
fn session_prune_empty_defaults_to_dry_run_and_requires_force_to_delete() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let populated = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut renamed_populated = populated.clone();
    renamed_populated
        .rename("real task api_key = sk-list-secret")
        .unwrap();
    populated.append_message("user", "real task").unwrap();
    let empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut titled_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    titled_empty
        .rename("keep empty token = fake-redacted-marker")
        .unwrap();
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();

    let dry_run = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["prune-empty".into()],
    )
    .unwrap();
    assert!(dry_run.contains("would delete empty sessions: 1"));
    assert!(dry_run.contains(&format!("full={}", empty.id())));
    assert!(dry_run.contains("skipped titled empty sessions: 1"));
    assert!(dry_run.contains(&format!("full={}", titled_empty.id())));
    assert!(dry_run.contains("<redacted>"));
    assert!(!dry_run.contains("fake-redacted-marker"));
    assert!(dry_run.contains(&format!("full={}", current_empty.id())));
    assert!(empty.path().exists());

    let json_dry_run = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![
            "prune-empty".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/prune-empty.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&json_dry_run).unwrap();
    assert_eq!(value["schema"], "deepcli.session.prune_empty.v1");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["force"], false);
    assert_eq!(value["candidateCount"], 1);
    assert_eq!(value["deletedCount"], 0);
    assert_eq!(value["candidates"][0]["id"], empty.id().to_string());
    assert_eq!(
        value["skippedCurrent"]["id"],
        current_empty.id().to_string()
    );
    assert_eq!(value["skippedTitledCount"], 1);
    assert_eq!(
        value["nextActions"][0],
        "deepcli session prune-empty --force --json"
    );
    let next_actions = json_string_array(&value["nextActions"]);
    assert!(next_actions
        .iter()
        .any(|item| item == "deepcli session list --all --json"));
    assert!(next_actions
        .iter()
        .any(|item| item == "deepcli history --limit 20"));
    assert!(
            next_actions
                .iter()
                .all(|action| action.starts_with("deepcli ") && !action.starts_with("deepcli /")),
            "session prune-empty JSON nextActions should be directly executable commands: {next_actions:?}"
        );
    assert!(
            next_actions
                .iter()
                .all(|action| !action.starts_with("/session") && !action.contains("`/")),
            "session prune-empty JSON nextActions should not require parsing slash-command prose: {next_actions:?}"
        );
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Delete empty sessions".to_string()));
    assert!(checklist_labels.contains(&"List saved sessions".to_string()));
    assert!(!json_dry_run.contains("fake-redacted-marker"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/prune-empty.json")).unwrap();
    assert_eq!(written, json_dry_run);

    let forced = handle_session(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec!["prune-empty".into(), "--force".into()],
    )
    .unwrap();
    assert!(forced.contains("deleted empty sessions: 1"));
    assert!(!empty.path().exists());
    assert!(populated.path().exists());
    assert!(titled_empty.path().exists());
    assert!(current_empty.path().exists());
}

#[test]
fn session_prune_empty_rejects_unknown_options() {
    let dir = tempdir().unwrap();
    let error = handle_session(dir.path(), None, vec!["prune-empty".into(), "--now".into()])
        .unwrap_err()
        .to_string();

    assert!(error.contains("unsupported /session prune-empty option"));

    let traversal = handle_session(
        dir.path(),
        None,
        vec![
            "prune-empty".into(),
            "--json".into(),
            "--output".into(),
            "../prune-empty.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../prune-empty.json").exists());
}

#[tokio::test]
async fn session_restore_backup_dry_run_previews_without_writing() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.save_backup("src/lib.rs", "old content\n").unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "new content\n").unwrap();
    let executor = test_executor(dir.path());

    let output = handle_session_command(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["restore-backup".into(), "latest".into(), "--dry-run".into()],
    )
    .await
    .unwrap();

    assert!(output.contains("restore-backup dry-run"));
    assert!(output.contains("-new content"));
    assert!(output.contains("+old content"));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "new content\n"
    );
}

#[tokio::test]
async fn session_restore_backup_dry_run_json_writes_structured_preview() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_backup("src/lib.rs", "old content\napi_key = sk-restore-secret\n")
        .unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "new content\napi_key = sk-current-secret\n",
    )
    .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_session_command(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec![
            "restore-backup".into(),
            "latest".into(),
            "--dry-run".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/restore-preview.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.restore_backup.v1");
    assert_eq!(value["status"], "preview");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["session"]["id"], session.id().to_string());
    assert_eq!(value["backup"]["targetPath"], "src/lib.rs");
    assert!(value["target"]["path"]
        .as_str()
        .unwrap()
        .ends_with("src/lib.rs"));
    assert_eq!(value["target"]["workspacePath"], "src/lib.rs");
    assert!(value["diff"].as_str().unwrap().contains("-new content"));
    assert!(value["diff"].as_str().unwrap().contains("+old content"));
    assert!(!output.contains("sk-restore-secret"));
    assert!(!output.contains("sk-current-secret"));
    assert!(value["nextActions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str().unwrap().contains("restore-backup latest")));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/restore-preview.json")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), value);
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "new content\napi_key = sk-current-secret\n"
    );
}

#[tokio::test]
async fn session_restore_backup_writes_through_tool_executor_and_falls_back() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let backup_session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    backup_session
        .save_backup("src/lib.rs", "restored content\n")
        .unwrap();
    let current = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "current content\n").unwrap();
    let config = AppConfig::default();
    let permissions = PermissionEngine::new(
        dir.path(),
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        dir.path(),
        permissions,
        Some(current.clone()),
        config.agent.max_subagent_depth,
    );

    let output = handle_session_command(
        dir.path(),
        Some(current.id().to_string()),
        &executor,
        vec!["restore-backup".into(), "latest".into()],
    )
    .await
    .unwrap();

    assert!(output.contains("restored backup"));
    assert!(output.contains("latest session with backup records"));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "restored content\n"
    );
    assert_eq!(current.load_backups().unwrap().len(), 1);
    assert_eq!(current.load_diffs().unwrap().len(), 1);
}

#[test]
fn session_list_hides_empty_one_shot_sessions_by_default() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let older = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    older.append_message("user", "older task").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let populated = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let mut renamed_populated = populated.clone();
    renamed_populated
        .rename("real task api_key = sk-list-secret")
        .unwrap();
    populated.append_message("user", "real task").unwrap();
    let empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();

    let filtered = handle_session(dir.path(), None, vec!["list".into()]).unwrap();
    assert!(filtered.contains(&populated.id().to_string()));
    assert!(filtered.contains(&format!("id={}", short_id(&populated.id()))));
    assert!(filtered.contains(&format!("full={}", populated.id())));
    assert!(filtered.contains("<redacted>"));
    assert!(!filtered.contains("sk-list-secret"));
    assert!(!filtered.contains(&empty.id().to_string()));
    assert!(filtered.contains("hidden empty sessions: 1"));
    assert!(filtered.contains("/session list --all"));

    let limited = handle_session(
        dir.path(),
        None,
        vec!["list".into(), "--limit".into(), "1".into()],
    )
    .unwrap();
    assert!(limited.contains(&populated.id().to_string()));
    assert!(!limited.contains(&older.id().to_string()));
    assert!(!limited.contains(&empty.id().to_string()));
    assert!(limited.contains("showing 1/2 sessions"));
    assert!(limited.contains("hidden empty sessions: 1"));

    let all = handle_session(dir.path(), None, vec!["list".into(), "--all".into()]).unwrap();
    assert!(all.contains(&populated.id().to_string()));
    assert!(all.contains(&older.id().to_string()));
    assert!(all.contains(&empty.id().to_string()));
    assert!(!all.contains("hidden empty sessions"));

    let all_limited = handle_session(
        dir.path(),
        None,
        vec!["list".into(), "--all".into(), "--limit".into(), "2".into()],
    )
    .unwrap();
    assert!(all_limited.contains("showing 2/3 sessions"));
    assert!(!all_limited.contains("hidden empty sessions"));

    let json_output = handle_session(
        dir.path(),
        None,
        vec![
            "list".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/sessions.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(value["schema"], "deepcli.session.list.v1");
    assert_eq!(value["includeAll"], false);
    assert_eq!(value["matchingSessions"], 2);
    assert_eq!(value["shownSessions"], 2);
    assert_eq!(value["hiddenEmptySessions"], 1);
    assert!(value["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["metadata"]["id"] == populated.id().to_string()));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert!(next_actions.iter().any(|action| action
        == &format!(
            "deepcli resume {} --dry-run --json",
            short_id(&populated.id())
        )));
    assert!(next_actions.iter().any(|action| action
        == &format!(
            "deepcli session history {} --limit 20 --json",
            short_id(&populated.id())
        )));
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Resume preview".to_string()));
    assert!(checklist_labels.contains(&"Inspect session history".to_string()));
    assert!(checklist_labels.contains(&"Inspect recovery actions".to_string()));
    assert!(checklist_labels.contains(&"Inspect session diagnostics".to_string()));
    assert!(checklist_labels.contains(&"Open session help".to_string()));
    assert!(!json_output.contains("sk-list-secret"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/sessions.json")).unwrap();
    assert_eq!(written, json_output);

    let path_error = handle_session(
        dir.path(),
        None,
        vec!["list".into(), "--output".into(), "../sessions.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(path_error.contains("path traversal is not allowed"));
}

#[test]
fn approval_commands_find_pending_requests_across_one_shot_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let with_approval = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let request = with_approval
        .enqueue_bound_approval_request(
            "write_file",
            crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "write requires approval api_key = sk-approval-secret".to_string(),
            },
            "digest-write-command-test",
            "path=src/lib.rs",
            1,
        )
        .unwrap();
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let current_id = Some(current_empty.id().to_string());

    let list = handle_approval(dir.path(), current_id.clone(), vec!["list".into()]).unwrap();
    assert!(list.contains("latest session with pending approval requests"));
    assert!(list.contains("write_file"));
    assert!(list.contains("digest=digest-write-command-test"));
    assert!(list.contains("summary=path=src/lib.rs"));
    assert!(list.contains("confirmations=0/1"));
    assert!(list.contains("<redacted>"));
    assert!(!list.contains("sk-approval-secret"));

    let list_without_current = handle_approval(dir.path(), None, vec!["list".into()]).unwrap();
    assert!(list_without_current
        .contains("latest session with pending approval requests; no current session"));
    assert!(list_without_current.contains("write_file"));

    let list_json = handle_approval(
        dir.path(),
        current_id.clone(),
        vec![
            "list".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/approvals.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&list_json).unwrap();
    assert_eq!(value["schema"], "deepcli.approval.list.v1");
    assert_eq!(value["session"]["id"], with_approval.id().to_string());
    assert_eq!(value["itemCount"], 1);
    assert_eq!(value["pendingCount"], 1);
    assert_eq!(value["approvals"][0]["tool"], "write_file");
    assert_eq!(
        value["approvals"][0]["invocationDigest"],
        "digest-write-command-test"
    );
    assert_eq!(value["approvals"][0]["inputSummary"], "path=src/lib.rs");
    assert_eq!(value["approvals"][0]["confirmationsRequired"], 1);
    assert_eq!(value["approvals"][0]["confirmationsReceived"], 0);
    assert_eq!(value["approvals"][0]["approvedAt"], Value::Null);
    assert_eq!(value["approvals"][0]["consumedAt"], Value::Null);
    assert!(value["approvals"][0]["decision"]["reason"]
        .as_str()
        .unwrap()
        .contains("<redacted>"));
    assert!(!list_json.contains("sk-approval-secret"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Approve request".to_string()));
    assert!(checklist_labels.contains(&"Deny request".to_string()));
    assert!(checklist_labels.contains(&"Review approvals".to_string()));
    assert!(checklist_labels.contains(&"Open approval help".to_string()));
    let request_short_id = request.id.to_string()[..8].to_string();
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli approval approve {request_short_id}")));
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli approval deny {request_short_id}")));
    assert!(next_actions
        .iter()
        .any(|action| action
            == &format!("deepcli approval list {} --all --json", with_approval.id())));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli help approval"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/approvals.json")).unwrap();
    assert_eq!(written, list_json);

    let path_error = handle_approval(
        dir.path(),
        current_id.clone(),
        vec!["list".into(), "--output".into(), "../approvals.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(path_error.contains("path traversal is not allowed"));

    let current_list = handle_approval(
        dir.path(),
        current_id.clone(),
        vec!["list".into(), "--current".into()],
    )
    .unwrap();
    assert_eq!(current_list, "no approval requests");
    let current_list_json = handle_approval(
        dir.path(),
        current_id.clone(),
        vec!["list".into(), "--current".into(), "--json".into()],
    )
    .unwrap();
    let current_value: Value = serde_json::from_str(&current_list_json).unwrap();
    let current_next_actions = json_string_array(&current_value["nextActions"]);
    assert_executable_deepcli_actions(&current_next_actions);
    assert_checklist_matches_executable_actions(&current_value, &current_next_actions);
    let current_checklist_labels = json_checklist_labels(&current_value);
    assert!(current_checklist_labels.contains(&"Review approvals".to_string()));
    assert!(current_checklist_labels.contains(&"Open approval help".to_string()));
    assert!(current_next_actions
        .iter()
        .any(|action| action
            == &format!("deepcli approval list {} --all --json", current_empty.id())));
    assert!(current_next_actions
        .iter()
        .any(|action| action == "deepcli help approval"));
    assert!(!current_next_actions
        .iter()
        .any(|action| action.contains(" approve ")));
    assert!(!current_next_actions
        .iter()
        .any(|action| action.contains(" deny ")));

    let approved = handle_approval(
        dir.path(),
        current_id.clone(),
        vec!["approve".into(), request.id.to_string()[..8].to_string()],
    )
    .unwrap();
    assert!(approved.contains("approved request"));
    assert!(approved.contains("confirmations=1/1"));
    assert!(approved.contains("digest=digest-write-command-test"));
    assert!(approved.contains(&with_approval.id().to_string()));
    let loaded = store.load(&with_approval.id().to_string()).unwrap();
    assert_eq!(
        loaded.load_approval_requests().unwrap()[0].status,
        ApprovalStatus::Approved
    );

    let second_request = with_approval
        .enqueue_bound_approval_request(
            "run_shell",
            crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "shell requires approval api_key = sk-second-approval-secret".to_string(),
            },
            "digest-shell-command-test",
            "command=cargo test",
            2,
        )
        .unwrap();
    let first_confirmation = handle_approval(
        dir.path(),
        current_id.clone(),
        vec![
            "approve".into(),
            second_request.id.to_string()[..8].to_string(),
        ],
    )
    .unwrap();
    assert!(first_confirmation.contains("recorded confirmation for request"));
    assert!(first_confirmation.contains("confirmations=1/2"));
    let approved_json = handle_approval(
        dir.path(),
        current_id,
        vec![
            "approve".into(),
            second_request.id.to_string()[..8].to_string(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/approval-approve.json".into(),
        ],
    )
    .unwrap();
    let approved_value: Value = serde_json::from_str(&approved_json).unwrap();
    assert_eq!(approved_value["schema"], "deepcli.approval.action.v1");
    assert_eq!(approved_value["status"], "ok");
    assert_eq!(approved_value["action"], "approve");
    assert_eq!(
        approved_value["session"]["id"],
        with_approval.id().to_string()
    );
    assert_eq!(
        approved_value["approval"]["id"],
        second_request.id.to_string()
    );
    assert_eq!(approved_value["approval"]["status"], "approved");
    assert_eq!(
        approved_value["approval"]["invocationDigest"],
        "digest-shell-command-test"
    );
    assert_eq!(
        approved_value["approval"]["inputSummary"],
        "command=cargo test"
    );
    assert_eq!(approved_value["approval"]["confirmationsRequired"], 2);
    assert_eq!(approved_value["approval"]["confirmationsReceived"], 2);
    assert!(approved_value["approval"]["approvedAt"].is_string());
    assert_eq!(approved_value["approval"]["consumedAt"], Value::Null);
    assert!(!approved_json.contains("sk-second-approval-secret"));
    let approved_next_actions = json_string_array(&approved_value["nextActions"]);
    assert_executable_deepcli_actions(&approved_next_actions);
    assert_checklist_matches_executable_actions(&approved_value, &approved_next_actions);
    let approved_checklist_labels = json_checklist_labels(&approved_value);
    assert!(approved_checklist_labels.contains(&"Review approvals".to_string()));
    assert!(approved_checklist_labels.contains(&"Open approval help".to_string()));
    assert!(approved_next_actions
        .iter()
        .any(|action| action
            == &format!("deepcli approval list {} --all --json", with_approval.id())));
    let approved_written =
        fs::read_to_string(dir.path().join(".deepcli/exports/approval-approve.json")).unwrap();
    assert_eq!(approved_written, approved_json);

    with_approval
        .enqueue_bound_approval_request(
            "delete_file",
            crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "delete requires approval api_key = sk-clear-approval-secret".to_string(),
            },
            "digest-delete-command-test",
            "path=obsolete.txt",
            1,
        )
        .unwrap();
    let clear_json = handle_approval(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![
            "clear".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/approval-clear.json".into(),
        ],
    )
    .unwrap();
    let clear_value: Value = serde_json::from_str(&clear_json).unwrap();
    assert_eq!(clear_value["schema"], "deepcli.approval.action.v1");
    assert_eq!(clear_value["action"], "clear");
    assert_eq!(clear_value["session"]["id"], with_approval.id().to_string());
    assert_eq!(clear_value["approval"], Value::Null);
    assert_eq!(clear_value["clearedCount"], 1);
    assert!(!clear_json.contains("sk-clear-approval-secret"));
    let clear_next_actions = json_string_array(&clear_value["nextActions"]);
    assert_executable_deepcli_actions(&clear_next_actions);
    assert_checklist_matches_executable_actions(&clear_value, &clear_next_actions);
    let clear_checklist_labels = json_checklist_labels(&clear_value);
    assert!(clear_checklist_labels.contains(&"Review approvals".to_string()));
    assert!(clear_checklist_labels.contains(&"Open approval help".to_string()));
    let clear_written =
        fs::read_to_string(dir.path().join(".deepcli/exports/approval-clear.json")).unwrap();
    assert_eq!(clear_written, clear_json);
}

#[test]
fn btw_commands_find_open_questions_across_one_shot_sessions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let with_question = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    with_question.append_message("user", "main task").unwrap();
    let question = with_question
        .enqueue_side_question("explain later api_key = sk-btw-secret")
        .unwrap();
    let current_empty = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let current_id = Some(current_empty.id().to_string());

    let list = handle_btw(dir.path(), current_id.clone(), vec!["list".into()]).unwrap();
    assert!(list.contains("latest session with open side questions"));
    assert!(list.contains("explain later"));
    assert!(list.contains("<redacted>"));
    assert!(!list.contains("sk-btw-secret"));

    let list_without_current = handle_btw(dir.path(), None, vec!["list".into()]).unwrap();
    assert!(list_without_current
        .contains("latest session with open side questions; no current session"));
    assert!(list_without_current.contains("explain later"));

    let list_json = handle_btw(
        dir.path(),
        current_id.clone(),
        vec![
            "list".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/btw.json".into(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&list_json).unwrap();
    assert_eq!(value["schema"], "deepcli.btw.list.v1");
    assert_eq!(value["session"]["id"], with_question.id().to_string());
    assert_eq!(value["itemCount"], 1);
    assert_eq!(value["openCount"], 1);
    assert!(value["questions"][0]["question"]
        .as_str()
        .unwrap()
        .contains("explain later"));
    assert!(!list_json.contains("sk-btw-secret"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Review by-the-way questions".to_string()));
    assert!(checklist_labels.contains(&"Open by-the-way help".to_string()));
    assert!(next_actions
        .iter()
        .any(|action| action == &format!("deepcli btw list {} --all --json", with_question.id())));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli help btw"));
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/btw.json")).unwrap();
    assert_eq!(written, list_json);

    let path_error = handle_btw(
        dir.path(),
        current_id.clone(),
        vec!["list".into(), "--output".into(), "../btw.json".into()],
    )
    .unwrap_err()
    .to_string();
    assert!(path_error.contains("path traversal is not allowed"));

    let current_list = handle_btw(
        dir.path(),
        current_id.clone(),
        vec!["list".into(), "--current".into()],
    )
    .unwrap();
    assert_eq!(current_list, "no by-the-way questions");
    let current_list_json = handle_btw(
        dir.path(),
        current_id.clone(),
        vec!["list".into(), "--current".into(), "--json".into()],
    )
    .unwrap();
    let current_value: Value = serde_json::from_str(&current_list_json).unwrap();
    let current_next_actions = json_string_array(&current_value["nextActions"]);
    assert_executable_deepcli_actions(&current_next_actions);
    assert_checklist_matches_executable_actions(&current_value, &current_next_actions);
    let current_checklist_labels = json_checklist_labels(&current_value);
    assert!(current_checklist_labels.contains(&"Review by-the-way questions".to_string()));
    assert!(current_checklist_labels.contains(&"Open by-the-way help".to_string()));
    assert!(current_next_actions
        .iter()
        .any(|action| action == &format!("deepcli btw list {} --all --json", current_empty.id())));
    assert!(current_next_actions
        .iter()
        .any(|action| action == "deepcli help btw"));
    assert!(!current_next_actions
        .iter()
        .any(|action| action.contains(" answer ")));

    let answered = handle_btw(
        dir.path(),
        current_id.clone(),
        vec![
            "answer".into(),
            question.id.to_string()[..8].to_string(),
            "after".into(),
            "tests".into(),
        ],
    )
    .unwrap();
    assert!(answered.contains("answered by-the-way question"));
    assert!(answered.contains(&with_question.id().to_string()));
    let loaded = store.load(&with_question.id().to_string()).unwrap();
    assert_eq!(
        loaded.load_side_questions().unwrap()[0].status,
        SideQuestionStatus::Answered
    );

    let second_question = with_question
        .enqueue_side_question("pick model later api_key = sk-second-btw-secret")
        .unwrap();
    let answered_json = handle_btw(
        dir.path(),
        current_id.clone(),
        vec![
            "answer".into(),
            second_question.id.to_string()[..8].to_string(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/btw-answer.json".into(),
            "after".into(),
            "tests".into(),
        ],
    )
    .unwrap();
    let answered_value: Value = serde_json::from_str(&answered_json).unwrap();
    assert_eq!(answered_value["schema"], "deepcli.btw.action.v1");
    assert_eq!(answered_value["status"], "ok");
    assert_eq!(answered_value["action"], "answer");
    assert_eq!(
        answered_value["session"]["id"],
        with_question.id().to_string()
    );
    assert_eq!(
        answered_value["question"]["id"],
        second_question.id.to_string()
    );
    assert_eq!(answered_value["question"]["status"], "answered");
    assert_eq!(answered_value["question"]["answer"], "after tests");
    assert!(!answered_json.contains("sk-second-btw-secret"));
    let answered_next_actions = json_string_array(&answered_value["nextActions"]);
    assert_executable_deepcli_actions(&answered_next_actions);
    assert_checklist_matches_executable_actions(&answered_value, &answered_next_actions);
    let answered_checklist_labels = json_checklist_labels(&answered_value);
    assert!(answered_checklist_labels.contains(&"Review by-the-way questions".to_string()));
    assert!(answered_checklist_labels.contains(&"Open by-the-way help".to_string()));
    assert!(answered_next_actions
        .iter()
        .any(|action| action == &format!("deepcli btw list {} --all --json", with_question.id())));
    let answered_written =
        fs::read_to_string(dir.path().join(".deepcli/exports/btw-answer.json")).unwrap();
    assert_eq!(answered_written, answered_json);

    let queued = handle_btw(
        dir.path(),
        current_id,
        vec!["ask".into(), "follow-up".into(), "question".into()],
    )
    .unwrap();
    assert!(queued.contains(&with_question.id().to_string()));
    let reloaded = store.load(&with_question.id().to_string()).unwrap();
    assert_eq!(reloaded.load_side_questions().unwrap().len(), 3);

    let clear_json = handle_btw(
        dir.path(),
        Some(current_empty.id().to_string()),
        vec![
            "clear".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/btw-clear.json".into(),
        ],
    )
    .unwrap();
    let clear_value: Value = serde_json::from_str(&clear_json).unwrap();
    assert_eq!(clear_value["schema"], "deepcli.btw.action.v1");
    assert_eq!(clear_value["action"], "clear");
    assert_eq!(clear_value["session"]["id"], with_question.id().to_string());
    assert_eq!(clear_value["question"], Value::Null);
    assert_eq!(clear_value["clearedCount"], 1);
    let clear_next_actions = json_string_array(&clear_value["nextActions"]);
    assert_executable_deepcli_actions(&clear_next_actions);
    assert_checklist_matches_executable_actions(&clear_value, &clear_next_actions);
    let clear_checklist_labels = json_checklist_labels(&clear_value);
    assert!(clear_checklist_labels.contains(&"Review by-the-way questions".to_string()));
    assert!(clear_checklist_labels.contains(&"Open by-the-way help".to_string()));
    let clear_written =
        fs::read_to_string(dir.path().join(".deepcli/exports/btw-clear.json")).unwrap();
    assert_eq!(clear_written, clear_json);
}

#[test]
fn config_get_set_and_validate_use_project_config() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();

    let initial = handle_config(
        dir.path(),
        &config,
        vec![
            "get".to_string(),
            "agent.providerTurnTimeoutSeconds".to_string(),
        ],
    )
    .unwrap();
    assert_eq!(initial, "600");

    let updated = handle_config(
        dir.path(),
        &config,
        vec![
            "set".to_string(),
            "agent.providerTurnTimeoutSeconds".to_string(),
            "45".to_string(),
        ],
    )
    .unwrap();
    assert!(updated.contains("agent.providerTurnTimeoutSeconds = 45"));

    let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["agent"]["providerTurnTimeoutSeconds"], 45);

    let reloaded = AppConfig::load_effective(dir.path(), None).unwrap();
    let validation = handle_config(dir.path(), &reloaded, vec!["validate".to_string()]).unwrap();
    assert!(validation.contains("config validation: ok"));

    let get_json = handle_config(
        dir.path(),
        &reloaded,
        vec![
            "get".to_string(),
            "agent.providerTurnTimeoutSeconds".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/config-timeout.json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&get_json).unwrap();
    assert_eq!(value["schema"], "deepcli.config.inspect.v1");
    assert_eq!(value["kind"], "get");
    assert_eq!(value["path"], "agent.providerTurnTimeoutSeconds");
    assert_eq!(value["payload"], 45);
    assert_eq!(value["report"], "45");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Validate project config".to_string()));
    assert!(checklist_labels.contains(&"Inspect credentials".to_string()));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/config-timeout.json")).unwrap();
    assert_eq!(written, get_json);

    let validate_json = handle_config(
        dir.path(),
        &reloaded,
        vec!["validate".to_string(), "--json".to_string()],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&validate_json).unwrap();
    assert_eq!(value["schema"], "deepcli.config.inspect.v1");
    assert_eq!(value["kind"], "validate");
    assert_eq!(value["payload"]["valid"], true);
    assert_eq!(value["payload"]["defaultProvider"], "deepseek");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect credentials".to_string()));
    assert!(checklist_labels.contains(&"Inspect active model".to_string()));
}

#[test]
fn config_set_rejects_semantically_invalid_config() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let result = handle_config(
        dir.path(),
        &config,
        vec![
            "set".to_string(),
            "defaultProvider".to_string(),
            "missing".to_string(),
        ],
    );
    assert!(result.is_err());
    assert!(!dir.path().join(".deepcli/config.json").exists());
}

#[test]
fn timeout_command_shows_sets_resets_and_writes_json() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();

    let shown = handle_timeout(dir.path(), &config, Vec::new()).unwrap();
    assert!(shown.contains("provider turn timeout: 600s"));
    assert!(shown.contains("agent.providerTurnTimeoutSeconds"));

    let updated = handle_timeout(
        dir.path(),
        &config,
        vec![
            "45".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/timeout.json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&updated).unwrap();
    assert_eq!(value["schema"], "deepcli.timeout.v1");
    assert_eq!(value["action"], "set");
    assert_eq!(value["seconds"], 45);
    assert_eq!(value["path"], "agent.providerTurnTimeoutSeconds");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_eq!(next_actions[0], "deepcli usage --json");
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli timeout reset"));
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect session usage".to_string()));
    assert!(checklist_labels.contains(&"Reset provider timeout".to_string()));

    let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
    let config_value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(config_value["agent"]["providerTurnTimeoutSeconds"], 45);
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/timeout.json")).unwrap();
    assert_eq!(written, updated);

    let reloaded = AppConfig::load_effective(dir.path(), None).unwrap();
    let reset = handle_timeout(dir.path(), &reloaded, vec!["reset".to_string()]).unwrap();
    assert!(reset.contains("provider turn timeout: 600s"));
    let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
    let config_value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(config_value["agent"]["providerTurnTimeoutSeconds"], 600);
}

#[test]
fn timeout_command_rejects_invalid_values_and_path_traversal() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();

    let zero = handle_timeout(dir.path(), &config, vec!["0".to_string()])
        .unwrap_err()
        .to_string();
    assert!(zero.contains("greater than 0"));

    let traversal = handle_timeout(
        dir.path(),
        &config,
        vec![
            "--json".to_string(),
            "--output".to_string(),
            "../timeout.json".to_string(),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(traversal.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../timeout.json").exists());
}

#[test]
fn config_sources_reports_project_and_environment_inputs() {
    let dir = tempdir().unwrap();
    let output = handle_config(
        dir.path(),
        &AppConfig::default(),
        vec!["sources".to_string()],
    )
    .unwrap();
    assert!(output.contains("global config:"));
    assert!(output.contains("project config:"));
    assert!(output.contains("DEEPCLI_PROVIDER"));
    assert!(output.contains("DEEPSEEK_API_KEY"));

    let json_output = handle_config(
        dir.path(),
        &AppConfig::default(),
        vec![
            "sources".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/config-sources.json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(value["schema"], "deepcli.config.inspect.v1");
    assert_eq!(value["kind"], "sources");
    assert_eq!(value["payload"]["project"]["present"], false);
    assert!(value["payload"]["environment"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["key"] == "DEEPCLI_PROVIDER"));
    assert!(value["payload"]["providerApiKeys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["key"] == "DEEPSEEK_API_KEY"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Validate project config".to_string()));
    assert!(checklist_labels.contains(&"Inspect credentials".to_string()));
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/config-sources.json")).unwrap();
    assert_eq!(written, json_output);
}

#[test]
fn config_read_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let error = handle_config(
        dir.path(),
        &AppConfig::default(),
        vec![
            "show".to_string(),
            "--output".to_string(),
            "../config.json".to_string(),
        ],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../config.json").exists());
}

#[test]
fn permissions_show_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();

    let output = handle_permissions(
        dir.path(),
        &config,
        vec![
            "show".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/permissions.json".to_string(),
        ],
    )
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.permissions.show.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["effectiveMode"], "sandbox");
    assert_eq!(value["permissions"]["defaultMode"], "sandbox");
    assert_eq!(value["sandbox"]["enabledByDefault"], true);
    assert_eq!(value["capabilities"]["network"], true);
    assert_eq!(value["requiresApproval"]["workspaceWrite"], true);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Validate project config".to_string()));
    assert!(checklist_labels.contains(&"Open permissions help".to_string()));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("\"defaultMode\": \"sandbox\""));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/permissions.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn permissions_show_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let error = handle_permissions(
        dir.path(),
        &AppConfig::default(),
        vec![
            "show".to_string(),
            "--output".to_string(),
            "../permissions.json".to_string(),
        ],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../permissions.json").exists());
}

#[test]
fn credentials_remove_clears_api_key_and_preserves_metadata() {
    let dir = tempdir().unwrap();
    let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
    let config = test_provider_config(&provider);
    let path = dir
        .path()
        .join(format!(".deepcli/credentials/{provider}-credentials.json"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        serde_json::to_vec_pretty(&ProviderCredentials {
            provider: Some(provider.clone()),
            name: Some(provider.clone()),
            endpoint: Some("https://example.test/v1/chat".to_string()),
            model: Some("custom-model".to_string()),
            api_key: Some("remove-me-secret".to_string()),
            api_id: Some("account-id".to_string()),
            updated_at: None,
        })
        .unwrap(),
    )
    .unwrap();

    let output = handle_credentials(
        dir.path(),
        &config,
        vec!["remove".to_string(), provider.clone()],
    )
    .unwrap();
    assert!(output.contains("removed local apiKey"));
    assert!(!output.contains("remove-me-secret"));

    let raw = fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("remove-me-secret"));
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert!(value["apiKey"].is_null());
    assert_eq!(value["endpoint"], "https://example.test/v1/chat");
    assert_eq!(value["model"], "custom-model");
    assert_eq!(value["apiId"], "account-id");

    let status = handle_credentials(
        dir.path(),
        &config,
        vec!["status".to_string(), provider.clone()],
    )
    .unwrap();
    assert!(status.contains("api_key=missing"));
}

#[test]
fn credential_aliases_parse_to_local_credential_actions() {
    assert_eq!(
        CommandRouter::parse("/login deepseek --stdin").unwrap(),
        Some(SlashCommand::Credentials {
            args: vec![
                "set".to_string(),
                "deepseek".to_string(),
                "--stdin".to_string()
            ]
        })
    );
    assert_eq!(
        CommandRouter::parse("/apikey kimi").unwrap(),
        Some(SlashCommand::Credentials {
            args: vec!["set".to_string(), "kimi".to_string()]
        })
    );
    assert_eq!(
        CommandRouter::parse("/logout deepseek").unwrap(),
        Some(SlashCommand::Credentials {
            args: vec!["remove".to_string(), "deepseek".to_string()]
        })
    );
}

#[test]
fn credentials_set_rejects_hidden_prompt_when_interactive_prompts_disabled() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let error = handle_credentials_with_default(
        dir.path(),
        &config,
        vec!["set".to_string(), "deepseek".to_string()],
        None,
        false,
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("interactive credential input is disabled"));
}

#[test]
fn credentials_status_json_output_is_structured_redacted_and_written() {
    let dir = tempdir().unwrap();
    let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
    let config = test_provider_config(&provider);
    let env_key = provider_env_key(&provider);
    let secret = "sk-credentials-status-secret";
    std::env::set_var(&env_key, secret);

    let output = handle_credentials(
        dir.path(),
        &config,
        vec![
            "status".into(),
            provider.clone(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/credentials.json".into(),
        ],
    )
    .unwrap();
    std::env::remove_var(&env_key);

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.credentials.status.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["provider"], provider);
    assert_eq!(value["providerCount"], 1);
    assert_eq!(value["configuredProviders"], 1);
    assert_eq!(value["missingProviders"], 0);
    assert_eq!(value["providers"][0]["provider"], provider);
    assert_eq!(value["providers"][0]["status"], "configured");
    assert_eq!(value["providers"][0]["apiKey"], "configured");
    assert_eq!(value["providers"][0]["file"]["present"], false);
    assert_eq!(value["providers"][0]["environment"]["key"], env_key);
    assert_eq!(value["providers"][0]["environment"]["present"], true);
    assert_eq!(value["providers"][0]["model"], "test-model");
    assert!(!output.contains(secret));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Inspect active model".to_string()));
    assert!(checklist_labels.contains(&"Validate project config".to_string()));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/credentials.json")).unwrap();
    assert_eq!(written, output);

    let missing_dir = tempdir().unwrap();
    let missing_provider = format!("missingcred{}", uuid::Uuid::new_v4().simple());
    let missing_config = test_provider_config(&missing_provider);
    let missing_output = handle_credentials(
        missing_dir.path(),
        &missing_config,
        vec!["status".into(), missing_provider.clone(), "--json".into()],
    )
    .unwrap();
    let missing_value: Value = serde_json::from_str(&missing_output).unwrap();
    assert_eq!(missing_value["schema"], "deepcli.credentials.status.v1");
    assert_eq!(missing_value["provider"], missing_provider);
    assert_eq!(missing_value["configuredProviders"], 0);
    assert_eq!(missing_value["missingProviders"], 1);
    let missing_next_actions = json_string_array(&missing_value["nextActions"]);
    assert_executable_deepcli_actions(&missing_next_actions);
    assert_checklist_matches_executable_actions(&missing_value, &missing_next_actions);
    assert!(missing_next_actions
        .iter()
        .any(|action| action == &format!("deepcli credentials set {missing_provider}")));
    let missing_labels = json_checklist_labels(&missing_value);
    assert!(missing_labels.contains(&"Configure provider credentials".to_string()));
}

#[test]
fn credentials_status_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
    let config = test_provider_config(&provider);

    let error = handle_credentials(
        dir.path(),
        &config,
        vec![
            "status".into(),
            provider,
            "--output".into(),
            "../credentials.json".into(),
        ],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../credentials.json").exists());
}

#[test]
fn credentials_set_shared_writer_redacts_secret_and_preserves_metadata() {
    let dir = tempdir().unwrap();
    let provider = format!("credtest{}", uuid::Uuid::new_v4().simple());
    let config = test_provider_config(&provider);

    let output = set_credentials_api_key(
        dir.path(),
        &config,
        &provider,
        "direct-secret".to_string(),
        false,
        "unit-test",
    )
    .unwrap();
    assert!(output.contains("apiKey redacted"));
    assert!(!output.contains("direct-secret"));

    let path = dir
        .path()
        .join(format!(".deepcli/credentials/{provider}-credentials.json"));
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("direct-secret"));
    assert!(raw.contains("test-model"));

    let rejected = set_credentials_api_key(
        dir.path(),
        &config,
        &provider,
        "replacement-secret".to_string(),
        false,
        "unit-test",
    );
    assert!(rejected.is_err());

    let replaced = set_credentials_api_key(
        dir.path(),
        &config,
        &provider,
        "replacement-secret".to_string(),
        true,
        "unit-test",
    )
    .unwrap();
    assert!(replaced.contains("apiKey redacted"));
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("replacement-secret"));
}

#[test]
fn doctor_fix_creates_project_scaffold_and_gitignore_entries() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    let report = apply_doctor_fixes(dir.path(), &AppConfig::default()).unwrap();

    assert!(report
        .actions
        .iter()
        .any(|action| action.contains("created .deepcli/config.json")));
    for path in [
        ".deepcli/config.json",
        ".deepcli/credentials",
        ".deepcli/sessions",
        ".deepcli/logs",
        ".deepcli/prompts",
        ".deepcli/skills",
        ".deepcli/agents",
        ".deepcli/exports",
        ".deepcli/authorization.json",
    ] {
        assert!(dir.path().join(path).exists(), "{path} was not created");
    }

    let gitignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".deepcli/credentials/"));
    assert!(gitignore.contains(".deepcli/sessions/"));
    assert!(gitignore.contains(".deepcli/authorization.json"));

    let second = apply_doctor_fixes(dir.path(), &AppConfig::default()).unwrap();
    assert!(second.actions.is_empty());
    let gitignore_after = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert_eq!(gitignore_after.matches(".deepcli/credentials/").count(), 1);
}

fn test_provider_config(provider: &str) -> AppConfig {
    let mut providers = BTreeMap::new();
    providers.insert(
        provider.to_string(),
        ProviderConfig {
            provider_type: "deepseek".to_string(),
            credentials_file: PathBuf::from(format!(
                ".deepcli/credentials/{provider}-credentials.json"
            )),
            acceptance_model: Some("test-model".to_string()),
            capabilities: vec!["tool_calling".to_string()],
        },
    );
    AppConfig {
        default_provider: provider.to_string(),
        providers,
        ..AppConfig::default()
    }
}

const MISSING_TEST_PROVIDER: &str = "missing-provider-2f7c1e";

fn json_string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("expected array")
        .iter()
        .map(|item| item.as_str().expect("expected string").to_string())
        .collect()
}

fn json_checklist_labels(value: &Value) -> Vec<String> {
    value["checklist"]
        .as_array()
        .expect("expected checklist")
        .iter()
        .map(|item| item["label"].as_str().expect("expected label").to_string())
        .collect()
}

fn assert_benchmark_checklist_matches_executable_actions(value: &Value, actions: &[String]) {
    let checklist = value["checklist"]
        .as_array()
        .expect("benchmark JSON should expose checklist");
    let executable_actions = actions
        .iter()
        .filter(|action| {
            action.starts_with("deepcli ") && !action.contains('<') && !action.contains('>')
        })
        .collect::<Vec<_>>();
    assert_eq!(
        checklist.len(),
        executable_actions.len(),
        "benchmark checklist should mirror executable nextActions"
    );
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"].as_str().unwrap(), executable_actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
}

fn assert_benchmark_checklist_matches_next_actions(value: &Value) {
    let next_actions = json_string_array(&value["nextActions"]);
    assert_benchmark_checklist_matches_executable_actions(value, &next_actions);
}

fn assert_checklist_matches_executable_actions(value: &Value, actions: &[String]) {
    let checklist = value["checklist"]
        .as_array()
        .expect("JSON report should expose checklist");
    let executable_actions = actions
        .iter()
        .filter(|action| {
            (action.starts_with("deepcli ")
                || action.starts_with("cargo ")
                || action.starts_with("git ")
                || action.starts_with("cd ")
                || action.starts_with("mkdir ")
                || action.starts_with("chmod ")
                || action.starts_with("ln ")
                || action.starts_with("rm "))
                && !action.contains('<')
                && !action.contains('>')
        })
        .collect::<Vec<_>>();
    assert_eq!(
        checklist.len(),
        executable_actions.len(),
        "checklist should mirror executable nextActions"
    );
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"].as_str().unwrap(), executable_actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
}

fn assert_executable_deepcli_actions(actions: &[String]) {
    assert!(!actions.is_empty(), "expected at least one next action");
    for action in actions {
        assert!(
            action.starts_with("deepcli "),
            "next action should be a deepcli command: {action}"
        );
        assert!(
            !action.starts_with('/'),
            "next action should not be a slash command: {action}"
        );
        assert!(
            !action.contains("`/") && !action.starts_with("run `"),
            "next action should not contain slash-command prose: {action}"
        );
        assert!(
            !action.contains('<') && !action.contains('>'),
            "next action should not contain placeholders: {action}"
        );
    }
}

fn assert_executable_shell_actions(actions: &[String]) {
    assert!(!actions.is_empty(), "expected at least one next action");
    for action in actions {
        assert!(
            action.starts_with("deepcli ")
                || action.starts_with("cd ")
                || action.starts_with("mkdir ")
                || action.starts_with("chmod ")
                || action.starts_with("ln ")
                || action.starts_with("rm "),
            "next action should be an executable shell command: {action}"
        );
        assert!(
            !action.starts_with('/'),
            "next action should not be a slash command: {action}"
        );
        assert!(
            !action.contains("`/") && !action.starts_with("run `"),
            "next action should not contain slash-command prose: {action}"
        );
        assert!(
            !action.contains('<') && !action.contains('>'),
            "next action should not contain placeholders: {action}"
        );
    }
}

#[test]
fn doctor_next_actions_point_to_missing_default_provider_credentials() {
    let dir = tempdir().unwrap();
    let config = test_provider_config(MISSING_TEST_PROVIDER);
    let actions = doctor_next_actions(dir.path(), &config, None, &[]);
    assert_executable_deepcli_actions(&actions);
    assert!(actions.iter().any(|action| action == "deepcli quickstart"));
    assert!(actions
        .iter()
        .any(|action| action == "deepcli credentials set missing-provider-2f7c1e"));
    assert!(actions
        .iter()
        .any(|action| action == "deepcli install docker --smoke"));
}

#[tokio::test]
async fn doctor_quick_skips_environment_check_without_session_record() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let output = handle_doctor(
        dir.path(),
        &config,
        &executor,
        None,
        vec!["--quick".to_string()],
    )
    .await
    .unwrap();

    assert!(output.contains("deepcli doctor --quick"));
    assert!(output.contains(concat!("version: ", env!("CARGO_PKG_VERSION"))));
    assert!(output.contains("registered slash commands:"));
    assert!(output.contains("provider turn timeout: 600s"));
    assert!(output.contains("environment: skipped (--quick/--no-env)"));
    assert!(SessionStore::new(dir.path()).list().unwrap().is_empty());
}

#[tokio::test]
async fn doctor_json_output_is_structured_redacted_and_written() {
    let dir = tempdir().unwrap();
    let config = test_provider_config(MISSING_TEST_PROVIDER);
    let executor = test_executor(dir.path());
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            MISSING_TEST_PROVIDER.to_string(),
            Some("test-model".to_string()),
        )
        .unwrap();
    session
        .rename("doctor api_key = sk-doctor-session-secret")
        .unwrap();

    let output = handle_doctor(
        dir.path(),
        &config,
        &executor,
        None,
        vec![
            "--quick".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/doctor.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.doctor.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["version"]["package"], "deepcli");
    assert_eq!(value["version"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(value["version"]["commandCount"].as_u64().unwrap() > 0);
    assert_eq!(value["mode"]["quick"], true);
    assert_eq!(value["mode"]["probeProvider"], false);
    assert_eq!(value["projectConfig"]["present"], false);
    assert_eq!(value["authorization"]["present"], false);
    assert_eq!(value["gitIdentity"]["status"], "no_git");
    assert_eq!(value["config"]["defaultProvider"], MISSING_TEST_PROVIDER);
    assert_eq!(value["config"]["providerTurnTimeoutSeconds"], 600);
    assert!(value["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["name"] == MISSING_TEST_PROVIDER && item["apiKey"] == "missing" }));
    assert_eq!(value["environment"]["status"], "skipped");
    assert_eq!(value["sessions"]["total"], 1);
    assert!(value["sessions"]["latest"]["title"]
        .as_str()
        .unwrap()
        .contains("<redacted>"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli config validate"));
    assert!(!output.contains("sk-doctor-session-secret"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/doctor.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn doctor_shell_json_reports_install_health_without_environment_check() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let output = handle_doctor(
        dir.path(),
        &config,
        &executor,
        None,
        vec!["shell".into(), "--json".into()],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.doctor.v1");
    assert_eq!(value["mode"]["shell"], true);
    assert_eq!(value["mode"]["quick"], true);
    assert_eq!(value["environment"]["status"], "skipped");
    assert_eq!(value["shell"]["deepcli"]["name"], "deepcli");
    assert!(value["shell"]["deepcli"]["expectedWorkspacePaths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().ends_with("/scripts/deepcli")));
    assert_eq!(
        value["shell"]["legacyCommands"].as_array().unwrap().len(),
        2
    );
    assert_eq!(value["shell"]["completions"].as_array().unwrap().len(), 3);
    assert!(value["shell"]["report"]
        .as_str()
        .unwrap()
        .contains("shell install:"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_shell_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli completion install zsh --force"));
    let shell_next_actions = json_string_array(&value["shell"]["nextActions"]);
    assert_executable_shell_actions(&shell_next_actions);
    assert!(SessionStore::new(dir.path()).list().unwrap().is_empty());
}

#[test]
fn shell_doctor_distinguishes_current_workspace_command_from_external_command() {
    let workspace = tempdir().unwrap();
    let scripts_dir = workspace.path().join("scripts");
    fs::create_dir_all(&scripts_dir).unwrap();
    let launcher = scripts_dir.join("deepcli");
    write_test_executable(&launcher);

    let workspace_status = shell_command_status_in(
        "deepcli",
        std::slice::from_ref(&scripts_dir),
        &expected_deepcli_workspace_paths(workspace.path()),
    );
    assert_eq!(workspace_status.status, "found");
    assert_eq!(workspace_status.workspace_match, Some(true));
    assert!(format_shell_command_status(&workspace_status).contains("workspace command"));

    let external = tempdir().unwrap();
    let external_command = external.path().join("deepcli");
    write_test_executable(&external_command);
    let external_status = shell_command_status_in(
        "deepcli",
        &[external.path().to_path_buf()],
        &expected_deepcli_workspace_paths(workspace.path()),
    );
    assert_eq!(external_status.status, "found_external");
    assert_eq!(external_status.workspace_match, Some(false));

    let actions = doctor_shell_next_actions(workspace.path(), &external_status, &[], &[]);
    assert_executable_shell_actions(&actions);
    assert!(actions
        .iter()
        .any(|action| action.starts_with("mkdir -p ~/.local/bin && ln -sf ")));
}

fn write_test_executable(path: &Path) {
    fs::write(path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

#[tokio::test]
async fn doctor_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let error = handle_doctor(
        dir.path(),
        &config,
        &executor,
        None,
        vec!["--output".into(), "../doctor.json".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../doctor.json").exists());
}

#[tokio::test]
async fn global_diagnose_works_without_session_and_skips_environment_by_default() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let output = handle_diagnose(dir.path(), &config, &executor, None, Vec::new())
        .await
        .unwrap();

    assert!(output.contains("deepcli diagnose"));
    assert!(output.contains("workspace health:"));
    assert!(output.contains("deepcli doctor --quick"));
    assert!(output.contains("environment: skipped (--quick/--no-env)"));
    assert!(output.contains("session diagnosis:"));
    assert!(output.contains("skipped: missing session id"));
    assert!(output.contains("quick links:"));
    assert!(output.contains("/quickstart"));
    assert!(output.contains("/diagnose --full-env"));
    assert!(SessionStore::new(dir.path()).list().unwrap().is_empty());
}

#[tokio::test]
async fn global_diagnose_includes_latest_session_report_when_available() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session.rename("repair parser").unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "run_shell".to_string(),
            input: json!({"command": "cargo test"}),
            output: json!({"error": "failed"}),
            decision: None,
            status: ToolCallStatus::Failed,
            created_at: chrono::Utc::now(),
        })
        .unwrap();

    let output = handle_diagnose(
        dir.path(),
        &config,
        &executor,
        None,
        vec!["--limit".into(), "1".into()],
    )
    .await
    .unwrap();

    assert!(output.contains("session diagnosis:"));
    assert!(output.contains("repair parser"));
    assert!(output.contains("recent failed or denied tools: 1"));
    assert!(output.contains("tool=run_shell"));
    assert!(output.contains("/session diagnose"));
}

#[tokio::test]
async fn global_diagnose_json_output_is_structured_and_written() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let output = handle_diagnose(
        dir.path(),
        &config,
        &executor,
        None,
        vec![
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/diagnose.json".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.diagnose.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["mode"]["fullEnvironment"], false);
    assert_eq!(value["mode"]["probeProvider"], false);
    assert_eq!(value["mode"]["limit"], 5);
    assert!(value["workspaceHealth"]
        .as_str()
        .unwrap()
        .contains("deepcli doctor --quick"));
    assert!(value["sessionDiagnosis"]
        .as_str()
        .unwrap()
        .contains("skipped: missing session id"));
    assert!(value["report"].as_str().unwrap().contains("quick links:"));
    assert_eq!(value["supportBundle"], Value::Null);
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli diagnose --full-env --json"));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli support .deepcli/support/latest --json"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/diagnose.json")).unwrap();
    assert_eq!(written, output);
}

#[tokio::test]
async fn global_diagnose_bundle_writes_redacted_support_artifacts() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    session
        .rename("support api_key = sk-support-secret")
        .unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "run_shell".to_string(),
            input: json!({"command": "cargo test"}),
            output: json!({"apiKey": "sk-support-secret"}),
            decision: None,
            status: ToolCallStatus::Failed,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let before_sessions = store.list().unwrap().len();

    let output = handle_diagnose(
        dir.path(),
        &config,
        &executor,
        None,
        vec![
            "--json".into(),
            "--bundle".into(),
            ".deepcli/support/latest".into(),
        ],
    )
    .await
    .unwrap();

    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.diagnose.v1");
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli diagnose --json"));
    assert!(value["supportBundle"]["manifest"]
        .as_str()
        .unwrap()
        .ends_with(".deepcli/support/latest/manifest.json"));
    let files = value["supportBundle"]["files"].as_array().unwrap();
    for name in [
        "README.txt",
        "issue.md",
        "version.json",
        "diagnose.json",
        "quickstart.json",
        "status.json",
        "usage.json",
        "trace.json",
        "logs.json",
        "sessions.json",
    ] {
        assert!(files.iter().any(|file| file["name"] == name), "{name}");
        assert!(dir
            .path()
            .join(".deepcli/support/latest")
            .join(name)
            .exists());
    }

    let manifest =
        fs::read_to_string(dir.path().join(".deepcli/support/latest/manifest.json")).unwrap();
    let manifest_value: Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest_value["schema"], "deepcli.support_bundle.v1");
    assert_eq!(manifest_value["files"].as_array().unwrap().len(), 10);
    let manifest_next_actions = json_string_array(&manifest_value["nextActions"]);
    assert_executable_deepcli_actions(&manifest_next_actions);
    assert_checklist_matches_executable_actions(&manifest_value, &manifest_next_actions);
    assert!(manifest_next_actions
        .iter()
        .any(|action| action == "deepcli diagnose --json"));
    assert!(manifest_next_actions
        .iter()
        .any(|action| action
            == "deepcli diagnose --full-env --bundle .deepcli/support/latest --json"));

    let issue = fs::read_to_string(dir.path().join(".deepcli/support/latest/issue.md")).unwrap();
    assert!(issue.contains("# deepcli issue report"));
    assert!(issue.contains("deepcli version:"));
    assert!(issue.contains("default provider: deepseek"));
    assert!(issue.contains("## Attachments"));
    assert!(issue.contains("version.json"));
    assert!(issue.contains("diagnose.json"));
    assert!(issue.contains("logs.json"));
    assert!(!issue.contains("sk-support-secret"));

    let version =
        fs::read_to_string(dir.path().join(".deepcli/support/latest/version.json")).unwrap();
    let version_value: Value = serde_json::from_str(&version).unwrap();
    assert_eq!(version_value["schema"], "deepcli.version.v1");
    assert_eq!(version_value["package"], "deepcli");

    let sessions =
        fs::read_to_string(dir.path().join(".deepcli/support/latest/sessions.json")).unwrap();
    assert!(sessions.contains("<redacted>"));
    assert!(!sessions.contains("sk-support-secret"));
    assert_eq!(store.list().unwrap().len(), before_sessions);
}

#[tokio::test]
async fn global_diagnose_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let error = handle_diagnose(
        dir.path(),
        &config,
        &executor,
        None,
        vec!["--output".into(), "../diagnose.txt".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../diagnose.txt").exists());
}

#[tokio::test]
async fn global_diagnose_bundle_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let executor = test_executor(dir.path());

    let error = handle_diagnose(
        dir.path(),
        &config,
        &executor,
        None,
        vec!["--bundle".into(), "../support".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../support").exists());
}

#[test]
fn diagnose_options_parse_session_limit_and_provider_probe() {
    let parsed = parse_diagnose_options(
        &[
            "--probe-provider".into(),
            "--provider".into(),
            "kimi".into(),
            "--limit".into(),
            "7".into(),
            "--full-env".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/diagnose.json".into(),
            "--bundle".into(),
            ".deepcli/support/latest".into(),
        ],
        Some("active".into()),
    )
    .unwrap();
    assert!(parsed.full_environment);
    assert!(parsed.probe_provider);
    assert!(parsed.json_output);
    assert_eq!(parsed.provider.as_deref(), Some("kimi"));
    assert_eq!(parsed.limit, 7);
    assert_eq!(parsed.session_id.as_deref(), Some("active"));
    assert!(!parsed.explicit_session);
    assert_eq!(
        parsed.output_path.as_deref(),
        Some(".deepcli/exports/diagnose.json")
    );
    assert_eq!(
        parsed.bundle_dir.as_deref(),
        Some(".deepcli/support/latest")
    );

    let explicit = parse_diagnose_options(&["--current".into()], Some("active".into())).unwrap();
    assert_eq!(explicit.session_id.as_deref(), Some("active"));
    assert!(explicit.explicit_session);

    let error = parse_diagnose_options(&["--provider".into(), "kimi".into()], None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("requires `--probe-provider`"));
}

#[test]
fn doctor_next_actions_use_environment_recommendations() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let compiler_test = DiscoveredTestCommand {
        source: dir.path().join("Makefile"),
        command: "docker run --rm -v $PWD:/workspace maxxing/compiler-dev autotest -koopa -s lv1"
            .to_string(),
        requires_docker: true,
        available: Some(false),
        note: None,
    };
    let compiler_missing = EnvironmentReport {
        target: "compiler".to_string(),
        ready: false,
        checks: Vec::new(),
        recommended_action: Some("/install compiler --smoke".to_string()),
    };
    let actions = doctor_next_actions(
        dir.path(),
        &config,
        Some(&compiler_missing),
        std::slice::from_ref(&compiler_test),
    );
    assert_executable_deepcli_actions(&actions);
    assert!(actions
        .iter()
        .any(|action| action == "deepcli install compiler --smoke"));
    assert!(actions
        .iter()
        .any(|action| action == "deepcli compiler test --json"));

    let docker_missing = EnvironmentReport {
        target: "docker".to_string(),
        ready: false,
        checks: Vec::new(),
        recommended_action: Some("/install docker --smoke".to_string()),
    };
    let actions = doctor_next_actions(dir.path(), &config, Some(&docker_missing), &[]);
    assert_executable_deepcli_actions(&actions);
    assert!(actions
        .iter()
        .any(|action| action == "deepcli install docker --smoke"));

    let compiler_ready = EnvironmentReport {
        target: "compiler".to_string(),
        ready: true,
        checks: Vec::new(),
        recommended_action: None,
    };
    let actions = doctor_next_actions(
        dir.path(),
        &config,
        Some(&compiler_ready),
        std::slice::from_ref(&compiler_test),
    );
    assert_executable_deepcli_actions(&actions);
    assert!(actions
        .iter()
        .any(|action| action == "deepcli compiler test --json"));
}

#[test]
fn environment_plan_explains_setup_steps_risk_and_commands() {
    let report = EnvironmentReport {
        target: "compiler".to_string(),
        ready: false,
        checks: vec![
            test_environment_check("homebrew", true),
            test_environment_check("docker_cli", true),
            test_environment_check("colima", true),
            EnvironmentCheck {
                name: "docker_daemon".to_string(),
                available: false,
                version: None,
                detail: Some("daemon is not running\nextra detail".to_string()),
            },
            test_environment_check("compiler_dev_image", false),
        ],
        recommended_action: Some("/install compiler --smoke".to_string()),
    };
    let compiler_test = DiscoveredTestCommand {
        source: PathBuf::from("Makefile"),
        command: "docker run --rm maxxing/compiler-dev autotest -koopa -s lv1".to_string(),
        requires_docker: true,
        available: Some(false),
        note: None,
    };
    let plan = format_environment_plan(&report, &[compiler_test], true);
    assert!(plan.contains("environment plan target: compiler"));
    assert!(plan.contains("docker_daemon: missing - daemon is not running"));
    assert!(plan.contains("start Colima Docker runtime"));
    assert!(plan.contains("inspect or pull maxxing/compiler-dev"));
    assert!(plan.contains("run compiler-dev smoke container"));
    assert!(plan.contains("setup may install Docker/Colima"));
    assert!(plan.contains("/install compiler --smoke"));
    assert!(plan.contains("/compiler test --json"));
}

#[test]
fn environment_options_parse_json_output_and_reject_unsafe_paths() {
    let parsed = parse_env_options(
        &[
            "compiler".into(),
            "--smoke".into(),
            "--json".into(),
            "--output=.deepcli/exports/env-plan.json".into(),
        ],
        "auto",
        true,
        true,
        "environment plan",
    )
    .unwrap();
    assert_eq!(parsed.target, "compiler");
    assert!(parsed.smoke_test);
    assert!(parsed.json_output);
    assert_eq!(
        parsed.output_path.as_deref(),
        Some(".deepcli/exports/env-plan.json")
    );

    let error = parse_env_options(&["auto".into()], "docker", false, false, "environment test")
        .unwrap_err()
        .to_string();
    assert!(error.contains("target `auto` is not supported"));

    let error = parse_env_options(
        &["--output".into(), "../env.json".into()],
        "auto",
        true,
        false,
        "environment check",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("path traversal is not allowed"));
}

#[test]
fn environment_check_json_output_is_structured() {
    let dir = tempdir().unwrap();
    let report = EnvironmentReport {
        target: "docker".to_string(),
        ready: false,
        checks: vec![
            test_environment_check("docker_cli", true),
            EnvironmentCheck {
                name: "docker_daemon".to_string(),
                available: false,
                version: None,
                detail: Some("daemon is not running".to_string()),
            },
        ],
        recommended_action: Some("/install docker --smoke".to_string()),
    };

    let output =
        format_environment_check_json(dir.path(), &report, "environment target: docker").unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.env.inspect.v1");
    assert_eq!(value["kind"], "check");
    assert_eq!(value["status"], "needs_setup");
    assert_eq!(value["target"], "docker");
    assert_eq!(value["ready"], false);
    assert_eq!(value["checks"][0]["name"], "docker_cli");
    assert_eq!(value["checks"][1]["detail"], "daemon is not running");
    assert_eq!(value["recommendedAction"], "/install docker --smoke");
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli install docker --smoke"));
    assert!(next_actions
        .iter()
        .any(|action| { action.as_str().unwrap() == "deepcli doctor docker --json" }));
    assert!(
        next_actions
            .iter()
            .all(|action| !action.as_str().unwrap().starts_with("run `")),
        "environment JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Set up local environment".to_string()));
    assert!(checklist_labels.contains(&"Check Docker environment".to_string()));
}

#[test]
fn environment_plan_json_output_is_structured() {
    let dir = tempdir().unwrap();
    let report = EnvironmentReport {
        target: "compiler".to_string(),
        ready: false,
        checks: vec![
            test_environment_check("homebrew", true),
            test_environment_check("docker_cli", true),
            test_environment_check("colima", true),
            EnvironmentCheck {
                name: "docker_daemon".to_string(),
                available: false,
                version: None,
                detail: Some("daemon is not running".to_string()),
            },
            test_environment_check("compiler_dev_image", false),
        ],
        recommended_action: Some("/install compiler --smoke".to_string()),
    };
    let compiler_test = DiscoveredTestCommand {
        source: dir.path().join("online-doc/docs/lv1-main/testing.md"),
        command: "docker run --rm maxxing/compiler-dev autotest -koopa -s lv1".to_string(),
        requires_docker: true,
        available: Some(false),
        note: Some("compiler-dev Docker autotest command".to_string()),
    };
    let text = format_environment_plan(&report, std::slice::from_ref(&compiler_test), true);

    let output = format_environment_plan_json(
        dir.path(),
        &report,
        std::slice::from_ref(&compiler_test),
        true,
        &text,
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.env.inspect.v1");
    assert_eq!(value["kind"], "plan");
    assert_eq!(value["effectiveTarget"], "compiler");
    assert_eq!(value["smokeTest"], true);
    assert!(value["wouldRun"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step.as_str().unwrap().contains("start Colima")));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command.as_str().unwrap() == "/install compiler --smoke"));
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(next_actions
        .iter()
        .any(|action| { action.as_str().unwrap() == "deepcli install compiler --smoke" }));
    assert!(next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli compiler test --json"));
    assert!(
        next_actions
            .iter()
            .all(|action| !action.as_str().unwrap().starts_with("run `")),
        "environment JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Set up local environment".to_string()));
    assert!(checklist_labels.contains(&"Run environment test".to_string()));
    assert_eq!(
        value["compilerTest"]["command"],
        "docker run --rm maxxing/compiler-dev autotest -koopa -s lv1"
    );
}

#[test]
fn environment_setup_json_output_reports_actions() {
    let dir = tempdir().unwrap();
    let before = EnvironmentReport {
        target: "docker".to_string(),
        ready: false,
        checks: vec![test_environment_check("docker_daemon", false)],
        recommended_action: Some("/install docker --smoke".to_string()),
    };
    let after = EnvironmentReport {
        target: "docker".to_string(),
        ready: true,
        checks: vec![test_environment_check("docker_daemon", true)],
        recommended_action: None,
    };
    let setup = EnvironmentSetupResult {
        target: "docker".to_string(),
        before,
        actions: vec![CommandOutput {
            command: "colima start --runtime docker".to_string(),
            exit_code: Some(0),
            stdout: "started".to_string(),
            stderr: String::new(),
        }],
        after,
        ready: true,
    };

    let output = format_environment_setup_result_json(
        dir.path(),
        "setup",
        &setup,
        "environment setup target: docker",
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.env.inspect.v1");
    assert_eq!(value["kind"], "setup");
    assert_eq!(value["status"], "ready");
    assert_eq!(
        value["actions"][0]["command"],
        "colima start --runtime docker"
    );
    assert_eq!(value["actions"][0]["exitCode"], 0);
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(next_actions
        .iter()
        .any(|action| action.as_str().unwrap() == "deepcli test discover --json"));
    assert!(
        next_actions
            .iter()
            .all(|action| !action.as_str().unwrap().starts_with("run `")),
        "environment JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Discover test commands".to_string()));

    let output = format_environment_setup_result_json(
        dir.path(),
        "test",
        &setup,
        "environment test target: docker",
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(next_actions
        .iter()
        .any(|action| { action.as_str().unwrap() == "deepcli accept --env-check docker --json" }));
    assert!(next_actions
        .iter()
        .any(|action| { action.as_str().unwrap() == "deepcli gate --env-check docker --json" }));
    assert!(
        next_actions
            .iter()
            .all(|action| !action.as_str().unwrap().starts_with("run `")),
        "environment JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Run acceptance checks".to_string()));
    assert!(checklist_labels.contains(&"Run delivery gate".to_string()));
}

#[test]
fn environment_test_json_output_reports_acceptance_actions() {
    let dir = tempdir().unwrap();
    let raw = json!({
        "passed": true,
        "output": {
            "command": "docker run --rm hello-world",
            "exit_code": 0,
            "stdout": "ok",
            "stderr": ""
        }
    });

    let output =
        format_environment_test_run_json(dir.path(), "docker", &raw, "environment test").unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.env.inspect.v1");
    assert_eq!(value["kind"], "test");
    assert_eq!(value["status"], "ready");
    let next_actions = value["nextActions"].as_array().unwrap();
    assert!(next_actions
        .iter()
        .any(|action| { action.as_str().unwrap() == "deepcli accept --env-check docker --json" }));
    assert!(next_actions
        .iter()
        .any(|action| { action.as_str().unwrap() == "deepcli gate --env-check docker --json" }));
    assert!(
        next_actions
            .iter()
            .all(|action| !action.as_str().unwrap().starts_with("run `")),
        "environment JSON nextActions should be directly executable commands: {next_actions:?}"
    );
    let next_action_strings = json_string_array(&value["nextActions"]);
    assert_checklist_matches_executable_actions(&value, &next_action_strings);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Run acceptance checks".to_string()));
    assert!(checklist_labels.contains(&"Run delivery gate".to_string()));
}

fn test_environment_check(name: &str, available: bool) -> EnvironmentCheck {
    EnvironmentCheck {
        name: name.to_string(),
        available,
        version: None,
        detail: None,
    }
}

#[test]
fn doctor_provider_readiness_reports_offline_state() {
    let dir = tempdir().unwrap();
    let config = test_provider_config(MISSING_TEST_PROVIDER);
    let reports = provider_readiness_reports(dir.path(), &config);
    let provider = reports
        .iter()
        .find(|report| report.name == MISSING_TEST_PROVIDER)
        .unwrap();
    assert_eq!(provider.credentials, "missing");
    assert_eq!(provider.model, "test-model");
    assert!(provider.implemented);
    assert!(provider
        .display()
        .contains("endpoint=https://api.deepseek.com/chat/completions"));
}

#[test]
fn doctor_options_require_probe_for_provider_selection() {
    assert_eq!(
        parse_doctor_options(&[
            "--probe-provider".into(),
            "--provider".into(),
            "kimi".into(),
            "--json".into(),
            "--output".into(),
            ".deepcli/exports/doctor.json".into()
        ])
        .unwrap(),
        DoctorOptions {
            fix: false,
            probe_provider: true,
            provider: Some("kimi".to_string()),
            shell_check: false,
            skip_environment: false,
            json_output: true,
            output_path: Some(".deepcli/exports/doctor.json".to_string()),
        }
    );
    assert_eq!(
        parse_doctor_options(&["--fix".into(), "--quick".into()]).unwrap(),
        DoctorOptions {
            fix: true,
            probe_provider: false,
            provider: None,
            shell_check: false,
            skip_environment: true,
            json_output: false,
            output_path: None,
        }
    );
    assert_eq!(
        parse_doctor_options(&["--no-env".into()]).unwrap(),
        DoctorOptions {
            fix: false,
            probe_provider: false,
            provider: None,
            shell_check: false,
            skip_environment: true,
            json_output: false,
            output_path: None,
        }
    );
    assert_eq!(
        parse_doctor_options(&["shell".into(), "--json".into()]).unwrap(),
        DoctorOptions {
            fix: false,
            probe_provider: false,
            provider: None,
            shell_check: true,
            skip_environment: true,
            json_output: true,
            output_path: None,
        }
    );
    assert!(parse_doctor_options(&["--provider".into(), "kimi".into()]).is_err());
}

#[tokio::test]
async fn doctor_provider_probe_skips_missing_credentials() {
    let dir = tempdir().unwrap();
    let config = test_provider_config(MISSING_TEST_PROVIDER);
    let report = probe_provider(dir.path(), &config, Some(MISSING_TEST_PROVIDER))
        .await
        .unwrap();
    assert_eq!(report.provider, MISSING_TEST_PROVIDER);
    assert_eq!(report.status, "skipped");
    assert!(report.message.contains("MISSING_PROVIDER_2F7C1E_API_KEY"));
    assert!(report
        .display()
        .contains("missing-provider-2f7c1e: skipped"));
}

#[test]
fn records_provider_probe_for_session_trace() {
    let dir = tempdir().unwrap();
    let session = SessionStore::new(dir.path())
        .create(
            dir.path(),
            "deepseek".to_string(),
            Some("deepseek-v4-pro".to_string()),
        )
        .unwrap();
    let report = ProviderProbeReport {
        provider: "deepseek".to_string(),
        status: "skipped".to_string(),
        elapsed_ms: Some(1),
        message: "api_key missing".to_string(),
        content_preview: None,
    };

    record_provider_probe(dir.path(), &session.id().to_string(), &report).unwrap();
    let loaded = SessionStore::new(dir.path())
        .load(&session.id().to_string())
        .unwrap();
    let trace = format_audit_trace(&loaded.load_audit_events().unwrap(), 10);
    assert!(trace.contains("provider_probe provider=deepseek status=skipped"));
}

#[test]
fn review_diff_flags_sensitive_additions() {
    let report = review_diff("+api_key = secret\n");
    assert!(report.contains("high:"));
    assert!(report.contains("sensitive"));
    assert!(report.contains("+api_key = <redacted>"));
    assert!(!report.contains("secret"));
}

#[test]
fn review_diff_deduplicates_repeated_findings() {
    let report = review_diff("+api_key = one\n+api_key = two\n+api_key = three\n");
    assert_eq!(
        report
            .matches("added line appears to contain sensitive material")
            .count(),
        1
    );
    assert!(report.contains("(3 occurrences)"));
    assert_eq!(report.matches("example:").count(), 3);
    assert!(!report.contains("one"));
    assert!(!report.contains("two"));
    assert!(!report.contains("three"));
}

#[test]
fn review_diff_flags_real_sensitive_source_values() {
    let report = review_diff(
        "diff --git a/src/lib.rs b/src/lib.rs\n+const API_KEY: &str = \"sk-real-example\";\n",
    );
    assert!(report.contains("sensitive material"));
    assert!(report.contains("<redacted"));
    assert!(!report.contains("sk-real-example"));
}

#[test]
fn review_diff_ignores_sensitive_labels_in_source_code() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+format!(\"authorization: {}\", status)\n+format!(\"api_key={}\", status)\n+\"printf '%s' \\\"$DEEPSEEK_API_KEY\\\" | /credentials set deepseek --stdin --force\"\n+let mut file_api_key = false;\n+if file_api_key || env_present { \"configured\" } else { \"missing\" }\n+file_api_key = credentials.api_key.is_some();\n+api_key: Some(format!(\"<replace locally>\")),\n+api_key: None,\n+api_key: String,\n+credentials.api_key = Some(api_key);\n+io::stdin().read_line(&mut api_key)?;\n+lines.push(\"provider API keys: DEEPSEEK_API_KEY, KIMI_API_KEY\".to_string());\n+format!(\"{}_API_KEY\", provider)\n+provider_env_key(provider)\n+api_key,\n+if has_explicit_secret_review_marker(text) { return true; }\n+let defines_api_key_rule = lower.contains(\"api_key\");\n+lower.contains(\"sk-\") || lower.contains(\"bearer \")\n+const SENSITIVE_HEADER_MARKERS: &[&str] = &[\"authorization:\"];\n+const SECRET_VALUE_MARKERS: &[&str] = &[\"bearer \", \"-----BEGIN PRIVATE KEY-----\"];\n+privacy_has_secret_value_marker(text)\n+fn has_secret_value_marker(text: &str) -> bool {\n+fn has_sensitive_header_marker(text: &str) -> bool {\n+fn contains_sk_secret_marker(lower: &str) -> bool {\n",
        );
    assert!(!report.contains("sensitive material"), "{report}");
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_task_oriented_recipe_help_text() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+                                   Show task-oriented workflow command recipes\n+            summary: \"Show task-oriented command recipes for common deepcli workflows.\",\n+            notes: &[\"`/recipes` is a local command catalog for task-oriented workflows.\"],\n",
        );
    assert!(!report.contains("sensitive material"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_sensitive_examples_in_docs() {
    let report = review_diff(
            "diff --git a/docs/setup.md b/docs/setup.md\n+api_key = secret\n+Authorization: Bearer example\n",
        );
    assert!(!report.contains("sensitive material"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_sensitive_examples_after_test_marker() {
    let report = review_diff(
            "diff --git a/src/privacy.rs b/src/privacy.rs\n+#[test]\n+fn sample() {\n+    let secret = \"test-secret-value\";\n+    assert_eq!(redact_sensitive_text(\"api_key = abc123\"), \"api_key = <redacted>\");\n+}\n",
        );
    assert!(!report.contains("sensitive material"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_removed_dangerous_commands() {
    let report = review_diff("diff --git a/scripts/setup.sh b/scripts/setup.sh\n-rm -rf target\n");
    assert!(!report.contains("dangerous command pattern"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_flags_added_shell_dangerous_commands() {
    let report = review_diff("diff --git a/scripts/setup.sh b/scripts/setup.sh\n+rm -rf target\n");
    assert!(report.contains("dangerous command pattern"));
    assert!(report.contains("+rm -rf target"));
}

#[test]
fn review_diff_ignores_detector_string_literals() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+if line.contains(\"rm -rf\") { return true; }\n",
        );
    assert!(!report.contains("dangerous command pattern"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_detector_contains_checks() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+text.contains(\"rm -rf\") || text.contains(\"git reset --hard\")\n",
        );
    assert!(!report.contains("dangerous command pattern"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_test_unwraps() {
    let report = review_diff(
            "diff --git a/tests/wrapper_contract.rs b/tests/wrapper_contract.rs\n+let value = result.unwrap();\n",
        );
    assert!(!report.contains("panic-prone"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_unwraps_after_test_marker() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+#[test]\n+fn review_case() {\n+    let value = result.unwrap();\n+}\n",
        );
    assert!(!report.contains("panic-prone"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_unwraps_in_mod_tests_hunk() {
    let report = review_diff(
            "diff --git a/src/session.rs b/src/session.rs\n@@ -10,3 +10,6 @@ mod tests {\n+let loaded = store.load(&id).unwrap();\n",
        );
    assert!(!report.contains("panic-prone"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_documented_invariant_expect() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+let latest = items.last().expect(\"items checked as non-empty\");\n",
        );
    assert!(!report.contains("panic-prone"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_flags_unexplained_expect() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+let latest = items.last().expect(\"latest item\");\n",
        );
    assert!(report.contains("panic-prone"));
}

#[test]
fn review_diff_ignores_panic_detector_literals() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+text.contains(\"unwrap()\") || text.contains(\"expect(\")\n",
        );
    assert!(!report.contains("panic-prone"));
    assert!(report.contains("low:"));
}

#[test]
fn review_diff_ignores_dangerous_strings_after_test_marker() {
    let report = review_diff(
            "diff --git a/src/commands.rs b/src/commands.rs\n+#[test]\n+fn review_case() {\n+    let sample = \"rm -rf target\";\n+}\n",
        );
    assert!(!report.contains("dangerous command pattern"));
    assert!(report.contains("low:"));
}

#[test]
fn review_risk_summary_counts_high_and_medium_sections() {
    let report = review_diff("+api_key = one\n+let value = maybe.unwrap();\n");
    let summary = review_risk_summary_from_report(&report);
    assert_eq!(summary.high_findings, 1);
    assert_eq!(summary.medium_findings, 1);
}

#[test]
fn review_worktree_reports_untracked_files() {
    let report = review_worktree("?? src/main.rs\n?? Cargo.toml\n", "");
    assert!(report.contains("untracked files: 2"));
    assert!(report.contains("src/main.rs"));
}

#[test]
fn filter_diff_by_paths_keeps_matching_file_sections() {
    let diff = "\
diff --git a/src/keep.rs b/src/keep.rs
+keep
diff --git a/docs/skip.md b/docs/skip.md
+skip
";
    let filtered = filter_diff_by_paths(diff, &["src".to_string()]);

    assert!(filtered.contains("src/keep.rs"));
    assert!(filtered.contains("+keep"));
    assert!(!filtered.contains("docs/skip.md"));
    assert!(!filtered.contains("+skip"));
}

#[test]
fn web_search_query_parses_search_alias_and_default_form() {
    assert_eq!(
        web_search_query_from_args(&[
            "search".to_string(),
            "rust".to_string(),
            "ownership".to_string()
        ])
        .unwrap(),
        "rust ownership"
    );
    assert_eq!(
        web_search_query_from_args(&["sysy".to_string(), "compiler".to_string()]).unwrap(),
        "sysy compiler"
    );
    assert!(web_search_query_from_args(&["search".to_string()]).is_err());
}

#[tokio::test]
async fn diff_falls_back_to_current_session_diffs_when_git_diff_is_unavailable() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.save_diff("src/lib.rs", "-old\n+new\n").unwrap();
    let executor = test_executor(dir.path());

    let output = handle_diff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        Vec::new(),
    )
    .await
    .unwrap();

    assert!(output.contains("session diff fallback"));
    assert!(output.contains(&session.id().to_string()));
    assert!(output.contains("+new"));
}

#[tokio::test]
async fn diff_falls_back_to_latest_session_with_diffs_when_current_has_none() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let diff_session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    diff_session
        .save_diff("src/lib.rs", "-old\n+new\n")
        .unwrap();
    let current = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_diff(
        dir.path(),
        Some(current.id().to_string()),
        &executor,
        Vec::new(),
    )
    .await
    .unwrap();

    assert!(output.contains("latest session with diff records"));
    assert!(output.contains(&diff_session.id().to_string()));
    assert!(output.contains("+new"));
}

#[tokio::test]
async fn staged_diff_keeps_git_semantics_without_session_fallback() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.save_diff("src/lib.rs", "-old\n+new\n").unwrap();
    let executor = test_executor(dir.path());

    let output = handle_diff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--staged".into()],
    )
    .await
    .unwrap();

    assert!(!output.contains("session diff fallback"));
    assert!(!output.contains("+new"));
}

#[tokio::test]
async fn diff_path_scope_filters_session_diff_fallback() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.save_diff("src/keep.rs", "+keep\n").unwrap();
    session.save_diff("docs/skip.md", "+skip\n").unwrap();
    let executor = test_executor(dir.path());

    let output = handle_diff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(output.contains("session diff fallback"));
    assert!(output.contains("scope: paths=src"));
    assert!(output.contains("+keep"));
    assert!(!output.contains("+skip"));
}

#[tokio::test]
async fn diff_stat_summarizes_scoped_session_diff_fallback() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/keep.rs", "-old\n+new\n+extra\n")
        .unwrap();
    session.save_diff("docs/skip.md", "+skip\n").unwrap();
    let executor = test_executor(dir.path());

    let output = handle_diff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--stat".into(), "--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(output.contains("session diff fallback"));
    assert!(output.contains("scope: paths=src"));
    assert!(output.contains("diff stat: 1 file(s), +2 -1"));
    assert!(output.contains("src_keep.rs +2 -1"));
    assert!(!output.contains("+new"));
    assert!(!output.contains("docs/skip.md"));
}

#[tokio::test]
async fn diff_limit_truncates_session_diff_fallback() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+one\n+two\n+three\n+four\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let output = handle_diff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--limit".into(), "3".into()],
    )
    .await
    .unwrap();

    assert!(output.contains("[deepcli session diff truncated"));
    assert!(output.contains("+one"));
    assert!(!output.contains("+four"));
}

#[tokio::test]
async fn review_falls_back_to_current_session_diffs_when_git_diff_is_unavailable() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+api_key = secret\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_review(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        Vec::new(),
    )
    .await
    .unwrap();

    assert!(report.contains("session diff review"));
    assert!(report.contains(&session.id().to_string()));
    assert!(report.contains("sensitive material"));
}

#[tokio::test]
async fn review_falls_back_to_latest_session_with_diffs_when_current_has_none() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let diff_session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    diff_session
        .save_diff("src/lib.rs", "+api_key = secret\n")
        .unwrap();
    let current = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_review(
        dir.path(),
        Some(current.id().to_string()),
        &executor,
        Vec::new(),
    )
    .await
    .unwrap();

    assert!(report.contains("latest session with diff records"));
    assert!(report.contains(&diff_session.id().to_string()));
    assert!(report.contains("sensitive material"));
}

#[tokio::test]
async fn review_path_scope_filters_session_diff_fallback() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/keep.rs", "+let ok = true;\n")
        .unwrap();
    session
        .save_diff("docs/skip.md", "+api_key = secret\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_review(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("scope: paths=src"));
    assert!(report.contains("session diff review"));
    assert!(!report.contains("docs/skip.md"));
    assert!(!report.contains("sensitive material"));
}

#[tokio::test]
async fn handoff_summarizes_session_diff_tests_and_next_actions() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .rename("compiler handoff")
        .expect("session title can be set");
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("handoff report"));
    assert!(report.contains("compiler handoff"));
    assert!(report.contains("scope: paths=src"));
    assert!(report.contains("diff stat: 1 file(s)"));
    assert!(report.contains("latest 1 recorded test run"));
    assert!(report.contains("risks and blockers:\n  none detected"));
    assert!(report.contains("/git message"));
}

#[tokio::test]
async fn handoff_markdown_formats_pr_ready_sections() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.rename("markdown handoff").unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--markdown".into(), "--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(report.starts_with("# deepcli Handoff"));
    assert!(report.contains("## Summary"));
    assert!(report.contains("## Changed Files"));
    assert!(report.contains("## Risks and Blockers"));
    assert!(report.contains("- workspace:"));
    assert!(report.contains("markdown handoff"));
    assert!(!report.contains("handoff report"));
}

#[tokio::test]
async fn handoff_pr_formats_pull_request_description() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.rename("pr handoff").unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--pr".into(), "--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(report.starts_with("<!-- generated by deepcli handoff --pr -->"));
    assert!(report.contains("## Summary"));
    assert!(report.contains("## Changes"));
    assert!(report.contains("## Test Plan"));
    assert!(report.contains("## Risks and Blockers"));
    assert!(report.contains("## Checklist"));
    assert!(report.contains("pr handoff"));
    assert!(report.contains("diff stat: 1 file(s)"));
    assert!(report.contains("No blockers detected by deepcli handoff"));
    assert!(!report.contains("handoff report"));
}

#[tokio::test]
async fn handoff_output_writes_selected_format_inside_workspace() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.rename("file handoff").unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec![
            "--pr".into(),
            "--output".into(),
            ".deepcli/exports/pr-description.md".into(),
        ],
    )
    .await
    .unwrap();
    let written =
        fs::read_to_string(dir.path().join(".deepcli/exports/pr-description.md")).unwrap();

    assert_eq!(written, report);
    assert!(written.starts_with("<!-- generated by deepcli handoff --pr -->"));
    assert!(written.contains("file handoff"));
}

#[tokio::test]
async fn handoff_output_writes_before_fail_on_blockers() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_handoff(
        dir.path(),
        None,
        &executor,
        vec![
            "--pr".into(),
            "--fail-on-blockers".into(),
            "--output".into(),
            "handoff/pr.md".into(),
        ],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    let written = fs::read_to_string(dir.path().join("handoff/pr.md")).unwrap();

    assert_eq!(exit.code, 1);
    assert_eq!(written, exit.output);
    assert!(written.contains("BLOCKER: no session context found"));
}

#[tokio::test]
async fn handoff_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_handoff(
        dir.path(),
        None,
        &executor,
        vec!["--output".into(), "../handoff.md".into()],
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("path traversal is not allowed"));
    assert!(!dir.path().join("../handoff.md").exists());
}

#[tokio::test]
async fn handoff_json_output_is_structured() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.rename("json handoff").unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--json".into(), "--path".into(), "src".into()],
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&report).unwrap();

    assert_eq!(value["schema"], "deepcli.handoff.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["hasBlockers"], false);
    assert_eq!(value["scope"], json!(["src"]));
    assert!(value["session"].as_str().unwrap().contains("json handoff"));
    assert!(value["diffSource"]
        .as_str()
        .unwrap()
        .contains("session diff fallback"));
    assert!(value["report"].as_str().unwrap().contains("handoff report"));
}

#[test]
fn handoff_report_includes_environment_evidence_in_text_pr_and_json() {
    let dir = tempdir().unwrap();
    let environment_checks = vec![VerificationEnvironmentCheck::Completed {
        target: "docker".to_string(),
        report: EnvironmentReport {
            target: "docker".to_string(),
            ready: false,
            checks: vec![
                test_environment_check("docker_cli", true),
                EnvironmentCheck {
                    name: "docker_daemon".to_string(),
                    available: false,
                    version: None,
                    detail: Some("daemon is not running".to_string()),
                },
            ],
            recommended_action: Some("/install docker --smoke".to_string()),
        },
        text: "environment target: docker\nready: false".to_string(),
    }];

    let report = format_handoff_report(HandoffReportInput {
        workspace: dir.path(),
        session: None,
        session_note: None,
        status: VerificationStatusSource {
            available: true,
            text: "",
            detail: None,
        },
        path_filters: &[],
        diff_source: VerificationDiffSource::None {
            git_available: true,
            detail: None,
        },
        limit: 5,
        environment_checks: &environment_checks,
    })
    .unwrap();

    assert!(report.contains("environment:"));
    assert!(report.contains("docker: [needs_setup] ready=false"));
    assert!(report.contains("missing checks: docker_daemon"));
    assert!(report.contains("environment `docker` is not ready"));
    assert!(report.contains("repair environment `docker`: `/install docker --smoke`"));

    let pr = format_handoff_report_pr_description(&report);
    assert!(pr.contains("## Environment"));
    assert!(pr.contains("docker: [needs_setup] ready=false"));
    assert!(pr.contains("BLOCKER: environment `docker` is not ready"));

    let json_output = format_handoff_report_json(&report, &environment_checks).unwrap();
    let value: Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(value["schema"], "deepcli.handoff.v1");
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["environment"]["requested"], true);
    assert_eq!(value["environment"]["targets"][0]["target"], "docker");
    assert_eq!(value["environment"]["targets"][0]["status"], "needs_setup");
}

#[tokio::test]
async fn handoff_fail_on_blockers_returns_report_with_command_exit() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_handoff(
        dir.path(),
        None,
        &executor,
        vec!["--fail-on-blockers".into()],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();

    assert_eq!(exit.code, 1);
    assert!(exit.output.contains("handoff report"));
    assert!(exit.output.contains("no session context found"));
    assert!(exit.output.contains("resolve blockers"));
}

#[tokio::test]
async fn handoff_json_fail_on_blockers_returns_structured_command_exit() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_handoff(
        dir.path(),
        None,
        &executor,
        vec!["--json".into(), "--fail-on-blockers".into()],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    let value: Value = serde_json::from_str(&exit.output).unwrap();

    assert_eq!(exit.code, 1);
    assert_eq!(value["schema"], "deepcli.handoff.v1");
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["hasBlockers"], true);
    assert!(value["blockers"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn handoff_fail_on_blockers_allows_clean_report() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--fail-on-blockers".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("risks and blockers:\n  none detected"));
    assert!(report.contains("/git message"));
}

#[tokio::test]
async fn handoff_reports_missing_evidence_as_blockers() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(dir.path(), None, &executor, Vec::new())
        .await
        .unwrap();

    assert!(report.contains("handoff report"));
    assert!(report.contains("session: none found"));
    assert!(report.contains("no session context found"));
    assert!(report.contains("no diff evidence found"));
    assert!(report.contains("resolve blockers"));
}

#[tokio::test]
async fn handoff_treats_smoke_only_recorded_test_as_blocker() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "printf ok".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        Vec::new(),
    )
    .await
    .unwrap();

    assert!(report.contains("evidence warning"));
    assert!(report.contains("no strong passing test evidence"));
    assert!(report.contains("add strong test evidence"));
    assert!(report.contains("resolve blockers"));
    assert!(!report.contains("none detected from recorded session signals"));
}

#[tokio::test]
async fn handoff_flags_stale_strong_test_evidence_after_diff() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now() - chrono::Duration::minutes(5),
        })
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_handoff(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("no strong passing test evidence after latest scoped diff change"));
    assert!(report.contains("add strong test evidence"));
    assert!(!report.contains("none detected from recorded session signals"));
}

#[tokio::test]
async fn verify_aggregates_session_diff_tests_and_blockers() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let mut session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session.rename("compiler verify").unwrap();
    session.set_state(SessionState::AwaitingApproval).unwrap();
    session
        .save_diff("src/lib.rs", "+api_key = secret\n+let ok = true;\n")
        .unwrap();
    session
        .append_tool_call(&ToolCallRecord {
            tool: "run_shell".to_string(),
            input: json!({"command": "cargo test"}),
            output: json!({"error": "tests failed"}),
            decision: None,
            status: ToolCallStatus::Failed,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(101),
            stdout: String::new(),
            stderr: "failed".to_string(),
            passed: false,
            created_at: chrono::Utc::now(),
        })
        .unwrap();
    session
        .enqueue_approval_request(
            "write_file",
            crate::permissions::PermissionDecision {
                outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                risk: crate::permissions::RiskLevel::High,
                reason: "write requires approval".to_string(),
            },
        )
        .unwrap();
    session
        .save_plan(&Plan {
            title: "verify plan".to_string(),
            steps: vec![PlanStep {
                id: "1".to_string(),
                description: "finish verification".to_string(),
                status: PlanStepStatus::Pending,
            }],
            updated_at: chrono::Utc::now(),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--limit".into(), "3".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("verification report"));
    assert!(report.contains("compiler verify"));
    assert!(report.contains("diff source: session diff fallback"));
    assert!(report.contains("sensitive material"));
    assert!(report.contains("latest 1 recorded test run"));
    assert!(report.contains("failed=1"));
    assert!(report.contains("pending approval request"));
    assert!(report.contains("failed or denied tool call"));
    assert!(report.contains("incomplete plan step"));
    assert!(report.contains("/session next"));
    assert!(report.contains("/review"));
}

#[tokio::test]
async fn verify_can_report_workspace_without_session() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(dir.path(), None, &executor, Vec::new())
        .await
        .unwrap();

    assert!(report.contains("verification report"));
    assert!(report.contains("session: none found"));
    assert!(report.contains("no session context found"));
    assert!(report.contains("no diff evidence found"));
}

#[tokio::test]
async fn verify_fail_on_blockers_returns_error_with_report() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_verify(
        dir.path(),
        None,
        &executor,
        vec!["--fail-on-blockers".into()],
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("verification report"));
    assert!(error.contains("blockers:"));
    assert!(error.contains("- no session context found"));
    assert!(error.contains("next actions:"));
}

#[tokio::test]
async fn verify_json_fail_on_blockers_returns_structured_command_exit() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_verify(
        dir.path(),
        None,
        &executor,
        vec!["--json".into(), "--fail-on-blockers".into()],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    assert_eq!(exit.code, 1);
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    assert_eq!(value["schema"], "deepcli.verify.v1");
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["hasBlockers"], true);
    assert!(value["blockers"][0]
        .as_str()
        .unwrap()
        .contains("no session context found"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("verification report"));
}

#[tokio::test]
async fn verify_json_next_actions_are_executable_commands() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_verify(
        dir.path(),
        None,
        &executor,
        vec!["--json".into(), "--fail-on-blockers".into()],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    let actions = value["nextActions"].as_array().unwrap();
    let checklist = value["checklist"].as_array().unwrap();

    assert!(!actions.is_empty(), "expected verify next actions");
    assert_eq!(checklist.len(), actions.len());
    for action in actions {
        let action = action.as_str().unwrap();
        assert!(
            action.starts_with("deepcli ")
                || action.starts_with("cargo ")
                || action.starts_with("git "),
            "verify next action should be directly executable: {action}"
        );
        assert!(
            !action.contains('`') && !action.starts_with("include "),
            "verify next action should not be explanatory prose: {action}"
        );
    }
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"], actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
    assert!(checklist
        .iter()
        .any(|item| item["label"] == "Record cargo test evidence"
            || item["label"] == "Run discovered tests"));
}

#[tokio::test]
async fn handoff_json_next_actions_are_executable_commands() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_handoff(
        dir.path(),
        None,
        &executor,
        vec!["--json".into(), "--fail-on-blockers".into()],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    let value: Value = serde_json::from_str(&exit.output).unwrap();
    let actions = value["nextActions"].as_array().unwrap();
    let checklist = value["checklist"].as_array().unwrap();

    assert!(!actions.is_empty(), "expected handoff next actions");
    assert_eq!(checklist.len(), actions.len());
    for action in actions {
        let action = action.as_str().unwrap();
        assert!(
            action.starts_with("deepcli ")
                || action.starts_with("cargo ")
                || action.starts_with("git "),
            "handoff next action should be directly executable: {action}"
        );
        assert!(
            !action.contains('`') && !action.contains('<'),
            "handoff next action should not contain prose markup or placeholders: {action}"
        );
    }
    for (index, item) in checklist.iter().enumerate() {
        assert_eq!(item["step"].as_u64().unwrap(), (index + 1) as u64);
        assert_eq!(item["command"], actions[index]);
        assert!(item["label"].as_str().unwrap().len() >= 3);
    }
    assert!(checklist
        .iter()
        .any(|item| item["label"] == "Prepare handoff report"
            || item["label"] == "Record cargo test evidence"));
}

#[tokio::test]
async fn verify_output_writes_selected_format_before_fail_on_blockers() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_verify(
        dir.path(),
        None,
        &executor,
        vec![
            "--json".into(),
            "--fail-on-blockers".into(),
            "--output".into(),
            ".deepcli/exports/verify.json".into(),
        ],
    )
    .await
    .unwrap_err();
    let exit = error.downcast_ref::<CommandExit>().unwrap();
    let written = fs::read_to_string(dir.path().join(".deepcli/exports/verify.json")).unwrap();
    let value: Value = serde_json::from_str(&written).unwrap();

    assert_eq!(exit.code, 1);
    assert_eq!(written, exit.output);
    assert_eq!(value["schema"], "deepcli.verify.v1");
    assert_eq!(value["status"], "blocked");
}

#[tokio::test]
async fn verify_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let executor = test_executor(dir.path());

    let error = handle_verify(
        dir.path(),
        None,
        &executor,
        vec!["--output".into(), "../verify.json".into()],
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("path traversal is not allowed"));
    assert!(!dir.path().join("../verify.json").exists());
}

#[test]
fn verify_report_includes_environment_evidence_and_json() {
    let dir = tempdir().unwrap();
    let environment_checks = vec![VerificationEnvironmentCheck::Completed {
        target: "docker".to_string(),
        report: EnvironmentReport {
            target: "docker".to_string(),
            ready: false,
            checks: vec![
                test_environment_check("docker_cli", true),
                EnvironmentCheck {
                    name: "docker_daemon".to_string(),
                    available: false,
                    version: None,
                    detail: Some("daemon is not running".to_string()),
                },
            ],
            recommended_action: Some("/install docker --smoke".to_string()),
        },
        text: "environment target: docker\nready: false".to_string(),
    }];

    let report = format_verification_report(VerificationReportInput {
        workspace: dir.path(),
        session: None,
        session_note: None,
        status: VerificationStatusSource {
            available: true,
            text: "",
            detail: None,
        },
        path_filters: &[],
        diff_source: VerificationDiffSource::None {
            git_available: true,
            detail: None,
        },
        test_limit: 5,
        test_run: VerificationTestRun::Completed {
            command: "cargo test".to_string(),
            passed: true,
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
        },
        environment_checks: &environment_checks,
    })
    .unwrap();

    assert!(report.contains("environment:"));
    assert!(report.contains("docker: [needs_setup] ready=false"));
    assert!(report.contains("missing checks: docker_daemon"));
    assert!(report.contains("environment `docker` is not ready"));
    assert!(report.contains("repair environment `docker`: `/install docker --smoke`"));
    assert!(!report.contains("- no session context found"));

    let output = format_verification_report_json(&report, &environment_checks).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.verify.v1");
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["environment"]["requested"], true);
    assert_eq!(value["environment"]["targets"][0]["target"], "docker");
    assert_eq!(value["environment"]["targets"][0]["status"], "needs_setup");
    assert_eq!(
        value["environment"]["targets"][0]["checks"][1]["name"],
        "docker_daemon"
    );
}

#[tokio::test]
async fn verify_workspace_only_allows_fresh_requested_strong_test_without_session_blocker() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn added() -> bool { ok() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::added()); }\n}\n",
        )
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        None,
        &executor,
        vec![
            "--path".into(),
            "src".into(),
            "--test-command".into(),
            "cargo test --quiet".into(),
        ],
    )
    .await
    .unwrap();

    assert!(report.contains("session evidence: none found; using workspace-only evidence"));
    assert!(report.contains("requested test run: [passed]"));
    assert!(report.contains("diff source: git diff scoped to src"));
    assert!(report.contains("blockers: none detected"));
    assert!(!report.contains("- no session context found"));
}

#[tokio::test]
async fn gate_without_current_session_ignores_stale_session_evidence_when_tests_pass() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    let store = SessionStore::new(dir.path());
    let stale = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    stale.append_message("user", "old failed task").unwrap();
    stale
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(101),
            stdout: String::new(),
            stderr: "old failure".to_string(),
            passed: false,
            created_at: chrono::Utc::now() - chrono::Duration::hours(1),
        })
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        None,
        &executor,
        vec![
            "--json".into(),
            "--run-tests".into(),
            "--fail-on-blockers".into(),
        ],
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&report).unwrap();

    assert_eq!(value["status"], "ok");
    assert_eq!(value["hasBlockers"], false);
    assert_eq!(value["blockers"].as_array().unwrap().len(), 0);
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("session evidence: none found; using workspace-only evidence"));
    assert!(!value["report"]
        .as_str()
        .unwrap()
        .contains("latest session with recorded activity"));
    assert!(!value["report"].as_str().unwrap().contains("old failure"));
}

#[tokio::test]
async fn verify_fail_on_blockers_allows_clean_workspace_only_report() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn added() -> bool { ok() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::added()); }\n}\n",
        )
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        None,
        &executor,
        vec![
            "--fail-on-blockers".into(),
            "--path".into(),
            "src".into(),
            "--test-command".into(),
            "cargo test --quiet".into(),
        ],
    )
    .await
    .unwrap();

    assert!(report.contains("blockers: none detected"));
}

#[tokio::test]
async fn verify_json_output_is_structured_for_clean_workspace_only_report() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    init_git_repo_with_baseline(dir.path());
    fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn ok() -> bool { true }\npub fn added() -> bool { ok() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() { assert!(super::added()); }\n}\n",
        )
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        None,
        &executor,
        vec![
            "--json".into(),
            "--fail-on-blockers".into(),
            "--path".into(),
            "src".into(),
            "--test-command".into(),
            "cargo test --quiet".into(),
        ],
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&report).unwrap();

    assert_eq!(value["schema"], "deepcli.verify.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["hasBlockers"], false);
    assert_eq!(value["blockers"].as_array().unwrap().len(), 0);
    assert_eq!(value["scope"][0], "src");
    assert!(value["diffSource"]
        .as_str()
        .unwrap()
        .contains("git diff scoped to src"));
    assert!(value["nextActions"].as_array().unwrap().len() >= 3);
}

#[tokio::test]
async fn verify_can_run_requested_tests_in_report() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec![
            "--test-command".into(),
            "cargo test --quiet".into(),
            "--limit".into(),
            "3".into(),
        ],
    )
    .await
    .unwrap();

    assert!(report.contains("requested test run: [passed]"));
    assert!(report.contains("command=cargo test --quiet"));
    assert!(report.contains("latest 1 recorded test run"));
    assert!(report.contains("blockers: none detected"));
    assert!(!report.contains("no test runs recorded for the selected session"));
}

#[tokio::test]
async fn verify_rejects_undiscovered_smoke_only_command() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--test-command".into(), "printf ok".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("requested test run: error"));
    assert!(report.contains("must extend a command discovered from the current workspace"));
    assert!(report.contains("requested verification test run failed"));
    assert!(!report.contains("blockers: none detected"));
}

#[tokio::test]
async fn verify_flags_stale_strong_test_evidence_after_scoped_diff() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .append_test_run(&TestRunRecord {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            passed: true,
            created_at: chrono::Utc::now() - chrono::Duration::minutes(5),
        })
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--path".into(), "src".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("evidence warning"));
    assert!(report.contains("no strong passing test evidence after latest scoped diff change"));
    assert!(report.contains("add strong test evidence"));
    assert!(!report.contains("blockers: none detected"));
}

#[tokio::test]
async fn verify_path_scope_filters_session_diff_fallback() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/keep.rs", "+let ok = true;\n")
        .unwrap();
    session
        .save_diff("docs/skip.md", "+api_key = secret\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec![
            "--path".into(),
            "src".into(),
            "--test-command".into(),
            "cargo test --quiet".into(),
        ],
    )
    .await
    .unwrap();

    assert!(report.contains("scope: paths=src"));
    assert!(report.contains("session diff fallback"));
    assert!(report.contains("with 1 record(s)"));
    assert!(!report.contains("docs/skip.md"));
    assert!(!report.contains("sensitive material"));
    assert!(report.contains("blockers: none detected"));
}

#[tokio::test]
async fn verify_failed_requested_test_is_a_blocker() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    fs::write(dir.path().join("src/lib.rs"), "this is not valid rust\n").unwrap();
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let ok = true;\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--test-command".into(), "cargo test --quiet".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("requested test run: [failed]"));
    assert!(report.contains("exit=Some(101)"));
    assert!(report.contains("requested output:"));
    assert!(report.contains("requested verification test run failed"));
}

#[tokio::test]
async fn verify_treats_high_review_risk_as_blocker() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+api_key = secret\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--test-command".into(), "cargo test --quiet".into()],
    )
    .await
    .unwrap();

    assert!(report.contains("auto-reviewer reported 1 high-risk finding type(s)"));
    assert!(!report.contains("blockers: none detected"));
}

#[tokio::test]
async fn verify_reports_medium_review_risk_as_warning_not_blocker() {
    let dir = tempdir().unwrap();
    write_minimal_cargo_project(dir.path());
    let store = SessionStore::new(dir.path());
    let session = store
        .create(dir.path(), "deepseek".to_string(), None)
        .unwrap();
    session
        .save_diff("src/lib.rs", "+let value = maybe.unwrap();\n")
        .unwrap();
    let executor = test_executor(dir.path());

    let report = handle_verify(
        dir.path(),
        Some(session.id().to_string()),
        &executor,
        vec!["--test-command".into(), "cargo test --quiet".into()],
    )
    .await
    .unwrap();

    assert!(
        report.contains("review warnings: auto-reviewer reported 1 medium-risk finding type(s)")
    );
    assert!(report.contains("blockers: none detected"));
    assert!(!report.contains("- auto-reviewer reported 1 medium-risk finding type(s)"));
}

#[test]
fn formats_side_questions_by_default_and_all() {
    let now = chrono::Utc::now();
    let open = SideQuestion {
        id: uuid::Uuid::new_v4(),
        question: "open item".to_string(),
        options: Vec::new(),
        answer: None,
        status: SideQuestionStatus::Open,
        created_at: now,
        updated_at: now,
    };
    let answered = SideQuestion {
        id: uuid::Uuid::new_v4(),
        question: "answered item".to_string(),
        options: Vec::new(),
        answer: Some("done".to_string()),
        status: SideQuestionStatus::Answered,
        created_at: now,
        updated_at: now,
    };
    let default = format_side_questions(&[open.clone(), answered.clone()], false);
    assert!(default.contains("open item"));
    assert!(!default.contains("answered item"));

    let all = format_side_questions(&[open, answered], true);
    assert!(all.contains("answered item"));
    assert!(all.contains("answer: done"));
}

#[test]
fn formats_approval_requests_by_default_and_all() {
    let now = chrono::Utc::now();
    let pending = ApprovalRequest {
        id: uuid::Uuid::new_v4(),
        tool: "write_file".to_string(),
        decision: crate::permissions::PermissionDecision {
            outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
            risk: crate::permissions::RiskLevel::High,
            reason: "write requires approval".to_string(),
        },
        status: ApprovalStatus::Pending,
        invocation_digest: Some("digest-write-format-test".to_string()),
        input_summary: Some("path=src/lib.rs".to_string()),
        confirmations_required: 1,
        confirmations_received: 0,
        approved_at: None,
        consumed_at: None,
        created_at: now,
        updated_at: now,
    };
    let approved = ApprovalRequest {
        id: uuid::Uuid::new_v4(),
        tool: "git_commit".to_string(),
        decision: crate::permissions::PermissionDecision {
            outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
            risk: crate::permissions::RiskLevel::High,
            reason: "git write requires approval".to_string(),
        },
        status: ApprovalStatus::Approved,
        invocation_digest: Some("digest-git-format-test".to_string()),
        input_summary: Some("git commit".to_string()),
        confirmations_required: 1,
        confirmations_received: 1,
        approved_at: Some(now),
        consumed_at: None,
        created_at: now,
        updated_at: now,
    };
    let default = format_approval_requests(&[pending.clone(), approved.clone()], false);
    assert!(default.contains("write_file"));
    assert!(default.contains("digest=digest-write-format-test"));
    assert!(default.contains("summary=path=src/lib.rs"));
    assert!(default.contains("confirmations=0/1"));
    assert!(!default.contains("git_commit"));

    let all = format_approval_requests(&[pending, approved], true);
    assert!(all.contains("git_commit"));
    assert!(all.contains("[approved]"));
}

#[test]
fn updates_project_model_config() {
    let dir = tempdir().unwrap();
    let deepcli = dir.path().join(".deepcli");
    fs::create_dir_all(&deepcli).unwrap();
    fs::write(
        deepcli.join("config.json"),
        r#"{
              "version": 1,
              "defaultProvider": "deepseek",
              "providers": {
                "deepseek": {
                  "type": "deepseek",
                  "credentialsFile": ".deepcli/credentials/deepseek-credentials.json",
                  "acceptanceModel": "old"
                }
              }
            }"#,
    )
    .unwrap();
    let config = AppConfig::load_effective(dir.path(), None).unwrap();
    update_project_model_config(dir.path(), &config, "deepseek", Some("deepseek-v4-pro")).unwrap();
    let raw = fs::read_to_string(deepcli.join("config.json")).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["defaultProvider"], "deepseek");
    assert_eq!(
        value["providers"]["deepseek"]["acceptanceModel"],
        "deepseek-v4-pro"
    );
}

#[test]
fn model_list_shows_configured_providers() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    let output = model_list_text(dir.path(), &config).unwrap();
    assert!(output.contains("* deepseek"));
    assert!(output.contains("kimi"));
    assert!(output.contains("model=deepseek-v4-pro"));
}

#[test]
fn model_show_json_output_is_structured_redacted_and_written() {
    let dir = tempdir().unwrap();
    let credentials_dir = dir.path().join(".deepcli/credentials");
    fs::create_dir_all(&credentials_dir).unwrap();
    fs::write(
        credentials_dir.join("deepseek-credentials.json"),
        r#"{
              "provider": "deepseek",
              "endpoint": "https://api.deepseek.example",
              "model": "deepseek-v4-pro",
              "apiKey": "sk-test-secret"
            }"#,
    )
    .unwrap();

    let output = handle_model(
        dir.path(),
        &AppConfig::default(),
        vec![
            "show".to_string(),
            "--json".to_string(),
            "--output".to_string(),
            ".deepcli/exports/model.json".to_string(),
        ],
    )
    .unwrap();

    assert!(!output.contains("sk-test-secret"));
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.model.inspect.v1");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["kind"], "show");
    assert_eq!(value["defaultProvider"], "deepseek");
    assert!(value["activeSession"].is_null());
    assert_eq!(value["provider"]["provider"], "deepseek");
    assert_eq!(value["provider"]["status"], "configured");
    assert_eq!(value["provider"]["apiKey"], "configured");
    assert_eq!(value["provider"]["credentials"]["present"], true);
    assert_eq!(value["provider"]["environment"]["key"], "DEEPSEEK_API_KEY");
    assert_eq!(value["provider"]["model"], "deepseek-v4-pro");
    assert!(value["provider"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "tool_calling"));
    assert!(value["report"]
        .as_str()
        .unwrap()
        .contains("default provider: deepseek"));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"List configured models".to_string()));
    assert!(checklist_labels.contains(&"Open model help".to_string()));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli model list --json"));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli help model"));

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/model.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn model_list_json_output_reports_providers_and_active_runtime_context() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();

    let output = handle_model(
        dir.path(),
        &config,
        vec![
            "list".to_string(),
            "--json".to_string(),
            "--output=.deepcli/exports/models.json".to_string(),
        ],
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], "deepcli.model.inspect.v1");
    assert_eq!(value["kind"], "list");
    assert_eq!(value["defaultProvider"], "deepseek");
    assert!(value["providerCount"].as_u64().unwrap() >= 2);
    assert!(value["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|provider| provider["provider"] == "deepseek" && provider["isDefault"] == true));
    let next_actions = json_string_array(&value["nextActions"]);
    assert_executable_deepcli_actions(&next_actions);
    assert_checklist_matches_executable_actions(&value, &next_actions);
    let checklist_labels = json_checklist_labels(&value);
    assert!(checklist_labels.contains(&"Switch configured model".to_string()));
    assert!(next_actions
        .iter()
        .any(|action| action == "deepcli model set kimi"));

    let active_output = handle_model_read_command(
        dir.path(),
        &config,
        &["show".to_string(), "--json".to_string()],
        Some(("deepseek", Some("deepseek-v4-pro"))),
    )
    .unwrap();
    let active_value: Value = serde_json::from_str(&active_output).unwrap();
    assert_eq!(active_value["activeSession"]["provider"], "deepseek");
    assert_eq!(active_value["activeSession"]["model"], "deepseek-v4-pro");

    let written = fs::read_to_string(dir.path().join(".deepcli/exports/models.json")).unwrap();
    assert_eq!(written, output);
}

#[test]
fn model_read_output_rejects_path_traversal() {
    let dir = tempdir().unwrap();
    let error = handle_model(
        dir.path(),
        &AppConfig::default(),
        vec![
            "list".to_string(),
            "--output".to_string(),
            "../models.json".to_string(),
        ],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("path traversal is not allowed"));
    assert!(!dir.path().join("../models.json").exists());
}

#[test]
fn model_set_rejects_option_shaped_provider_or_model() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();

    let provider_error = handle_model(
        dir.path(),
        &config,
        vec!["set".to_string(), "--json".to_string()],
    )
    .unwrap_err()
    .to_string();
    assert!(provider_error.contains("missing provider name"));

    let model_error = handle_model(
        dir.path(),
        &config,
        vec!["set".to_string(), "kimi".to_string(), "--json".to_string()],
    )
    .unwrap_err()
    .to_string();
    assert!(model_error.contains("usage: /model set <provider> [model]"));

    assert!(!dir.path().join(".deepcli/config.json").exists());
}

#[test]
fn update_project_model_config_creates_missing_project_config() {
    let dir = tempdir().unwrap();
    let config = AppConfig::default();
    update_project_model_config(dir.path(), &config, "deepseek", Some("deepseek-v4-pro")).unwrap();
    let raw = fs::read_to_string(dir.path().join(".deepcli/config.json")).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["defaultProvider"], "deepseek");
    assert_eq!(
        value["providers"]["deepseek"]["acceptanceModel"],
        "deepseek-v4-pro"
    );
}
