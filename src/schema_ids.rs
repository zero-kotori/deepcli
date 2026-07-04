//! Owner registry for stable JSON schema identifiers emitted by deepcli.
//!
//! Every user-visible `--json` payload carries a `"schema"` tag like
//! `deepcli.<name>.v1`. Centralizing the identifiers here gives each stable
//! schema a single owner and a single inventory, so emitters and consumers
//! reference the same constant instead of re-typing the literal in many places.

// Meta / diagnostics
pub const VERSION_V1: &str = "deepcli.version.v1";
pub const QUICKSTART_V1: &str = "deepcli.quickstart.v1";
pub const STATUS_V1: &str = "deepcli.status.v1";
pub const USAGE_V1: &str = "deepcli.usage.v1";
pub const TRACE_V1: &str = "deepcli.trace.v1";
pub const LOGS_V1: &str = "deepcli.logs.v1";
pub const NEXT_V1: &str = "deepcli.next.v1";
pub const TIMEOUT_V1: &str = "deepcli.timeout.v1";
pub const PERMISSIONS_SHOW_V1: &str = "deepcli.permissions.show.v1";
pub const CONFIG_INSPECT_V1: &str = "deepcli.config.inspect.v1";
pub const CREDENTIALS_STATUS_V1: &str = "deepcli.credentials.status.v1";
pub const MODEL_INSPECT_V1: &str = "deepcli.model.inspect.v1";
pub const SELFTEST_V1: &str = "deepcli.selftest.v1";
pub const PREFLIGHT_V1: &str = "deepcli.preflight.v1";
pub const DOCTOR_V1: &str = "deepcli.doctor.v1";
pub const DIAGNOSE_V1: &str = "deepcli.diagnose.v1";
pub const PRIVACY_SCAN_V1: &str = "deepcli.privacy.scan.v1";
pub const SUPPORT_BUNDLE_V1: &str = "deepcli.support_bundle.v1";
pub const SUPPORT_BUNDLE_ARTIFACT_V1: &str = "deepcli.support_bundle.artifact.v1";

// Shell completion
pub const COMPLETION_V1: &str = "deepcli.completion.v1";
pub const COMPLETION_INSTALL_V1: &str = "deepcli.completion.install.v1";
pub const COMPLETION_STATUS_V1: &str = "deepcli.completion.status.v1";

// Product loop
pub const SCORECARD_V1: &str = "deepcli.scorecard.v1";
pub const SCORECARD_SUMMARY_V1: &str = "deepcli.scorecard.summary.v1";
pub const ROUND_V1: &str = "deepcli.round.v1";
pub const RECIPES_V1: &str = "deepcli.recipes.v1";
pub const OPPORTUNITIES_V1: &str = "deepcli.opportunities.v1";

// Benchmark
pub const BENCHMARK_RECORD_V1: &str = "deepcli.benchmark.record.v1";
pub const BENCHMARK_SUITE_V1: &str = "deepcli.benchmark.suite.v1";
pub const BENCHMARK_STATUS_V1: &str = "deepcli.benchmark.status.v1";
pub const BENCHMARK_SUMMARY_V1: &str = "deepcli.benchmark.summary.v1";
pub const BENCHMARK_TRENDS_V1: &str = "deepcli.benchmark.trends.v1";
pub const BENCHMARK_BASELINE_V1: &str = "deepcli.benchmark.baseline.v1";
pub const BENCHMARK_BASELINES_V1: &str = "deepcli.benchmark.baselines.v1";
pub const BENCHMARK_COMPARE_V1: &str = "deepcli.benchmark.compare.v1";
pub const BENCHMARK_CLEANUP_V1: &str = "deepcli.benchmark.cleanup.v1";
pub const BENCHMARK_PRESETS_V1: &str = "deepcli.benchmark.presets.v1";
pub const BENCHMARK_LIST_V1: &str = "deepcli.benchmark.list.v1";

// Goal / plan
pub const GOAL_V1: &str = "deepcli.goal.v1";
pub const GOAL_STATUS_V1: &str = "deepcli.goal.status.v1";
pub const GOAL_STATUS_SUMMARY_V1: &str = "deepcli.goal.status.summary.v1";

// Session / resume / fork / terminal
pub const SESSION_LIST_V1: &str = "deepcli.session.list.v1";
pub const SESSION_SEARCH_V1: &str = "deepcli.session.search.v1";
pub const SESSION_INSPECT_V1: &str = "deepcli.session.inspect.v1";
pub const SESSION_DIAGNOSE_V1: &str = "deepcli.session.diagnose.v1";
pub const SESSION_PRUNE_EMPTY_V1: &str = "deepcli.session.prune_empty.v1";
pub const SESSION_RESTORE_BACKUP_V1: &str = "deepcli.session.restore_backup.v1";
pub const SESSION_FORK_V1: &str = "deepcli.session.fork.v1";
pub const RESUME_PREVIEW_V1: &str = "deepcli.resume.preview.v1";
pub const RESUME_CANDIDATES_V1: &str = "deepcli.resume.candidates.v1";
pub const TERMINAL_V1: &str = "deepcli.terminal.v1";

// Change delivery
pub const VERIFY_V1: &str = "deepcli.verify.v1";
pub const HANDOFF_V1: &str = "deepcli.handoff.v1";

// Git / environment / tests
pub const GIT_INSPECT_V1: &str = "deepcli.git.inspect.v1";
pub const GIT_ACTION_V1: &str = "deepcli.git.action.v1";
pub const ENV_INSPECT_V1: &str = "deepcli.env.inspect.v1";
pub const TEST_INSPECT_V1: &str = "deepcli.test.inspect.v1";

// Collaboration queues
pub const APPROVAL_LIST_V1: &str = "deepcli.approval.list.v1";
pub const APPROVAL_ACTION_V1: &str = "deepcli.approval.action.v1";
pub const BTW_LIST_V1: &str = "deepcli.btw.list.v1";
pub const BTW_ACTION_V1: &str = "deepcli.btw.action.v1";

// Local libraries
pub const PROMPT_INSPECT_V1: &str = "deepcli.prompt.inspect.v1";
pub const SKILL_INSPECT_V1: &str = "deepcli.skill.inspect.v1";
pub const AGENT_INSPECT_V1: &str = "deepcli.agent.inspect.v1";

/// Inventory of every stable schema identifier. The list doubles as the
/// reuse point for tooling that needs to enumerate stable schemas and keeps
/// each constant referenced from one place.
pub const ALL: &[&str] = &[
    VERSION_V1,
    QUICKSTART_V1,
    STATUS_V1,
    USAGE_V1,
    TRACE_V1,
    LOGS_V1,
    NEXT_V1,
    TIMEOUT_V1,
    PERMISSIONS_SHOW_V1,
    CONFIG_INSPECT_V1,
    CREDENTIALS_STATUS_V1,
    MODEL_INSPECT_V1,
    SELFTEST_V1,
    PREFLIGHT_V1,
    DOCTOR_V1,
    DIAGNOSE_V1,
    PRIVACY_SCAN_V1,
    SUPPORT_BUNDLE_V1,
    SUPPORT_BUNDLE_ARTIFACT_V1,
    COMPLETION_V1,
    COMPLETION_INSTALL_V1,
    COMPLETION_STATUS_V1,
    SCORECARD_V1,
    SCORECARD_SUMMARY_V1,
    ROUND_V1,
    RECIPES_V1,
    OPPORTUNITIES_V1,
    BENCHMARK_RECORD_V1,
    BENCHMARK_SUITE_V1,
    BENCHMARK_STATUS_V1,
    BENCHMARK_SUMMARY_V1,
    BENCHMARK_TRENDS_V1,
    BENCHMARK_BASELINE_V1,
    BENCHMARK_BASELINES_V1,
    BENCHMARK_COMPARE_V1,
    BENCHMARK_CLEANUP_V1,
    BENCHMARK_PRESETS_V1,
    BENCHMARK_LIST_V1,
    GOAL_V1,
    GOAL_STATUS_V1,
    GOAL_STATUS_SUMMARY_V1,
    SESSION_LIST_V1,
    SESSION_SEARCH_V1,
    SESSION_INSPECT_V1,
    SESSION_DIAGNOSE_V1,
    SESSION_PRUNE_EMPTY_V1,
    SESSION_RESTORE_BACKUP_V1,
    SESSION_FORK_V1,
    RESUME_PREVIEW_V1,
    RESUME_CANDIDATES_V1,
    TERMINAL_V1,
    VERIFY_V1,
    HANDOFF_V1,
    GIT_INSPECT_V1,
    GIT_ACTION_V1,
    ENV_INSPECT_V1,
    TEST_INSPECT_V1,
    APPROVAL_LIST_V1,
    APPROVAL_ACTION_V1,
    BTW_LIST_V1,
    BTW_ACTION_V1,
    PROMPT_INSPECT_V1,
    SKILL_INSPECT_V1,
    AGENT_INSPECT_V1,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn schema_ids_follow_stable_naming() {
        for id in ALL {
            assert!(
                id.starts_with("deepcli."),
                "{id} should be namespaced under deepcli."
            );
            let version = id.rsplit('.').next().unwrap_or_default();
            assert!(
                version.starts_with('v') && version[1..].chars().all(|c| c.is_ascii_digit()),
                "{id} should end with a versioned segment like v1"
            );
        }
    }

    #[test]
    fn schema_ids_are_unique() {
        let unique: HashSet<&&str> = ALL.iter().collect();
        assert_eq!(unique.len(), ALL.len(), "duplicate schema identifier");
    }
}
