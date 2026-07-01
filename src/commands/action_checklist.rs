use super::{
    generic_recipe_command_label, DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION,
    DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION, DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION,
};
use serde_json::{json, Value};

pub(crate) fn scorecard_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .filter(|action| action.starts_with("deepcli ") && !action.contains('<'))
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": scorecard_checklist_label(command),
                "command": command,
            })
        })
        .collect()
}

pub(crate) fn local_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
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
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": local_checklist_label(command),
                "command": command,
            })
        })
        .collect()
}

pub(crate) fn benchmark_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .filter(|action| {
            action.starts_with("deepcli ") && !action.contains('<') && !action.contains('>')
        })
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": benchmark_checklist_label(command),
                "command": command,
            })
        })
        .collect()
}

fn benchmark_checklist_label(command: &str) -> &'static str {
    if command.starts_with("deepcli benchmark compare --baseline") {
        "Compare benchmark baseline"
    } else if command.starts_with("deepcli benchmark clean --force") {
        "Delete benchmark artifacts"
    } else {
        match command {
            "deepcli benchmark list --json" => "List benchmark artifacts",
            "deepcli benchmark show latest --json" => "Show latest benchmark artifact",
            "deepcli benchmark clean --dry-run --json" => "Preview benchmark cleanup",
            "deepcli scorecard --json" => "Inspect product gaps",
            _ => scorecard_checklist_label(command),
        }
    }
}

fn local_checklist_label(command: &str) -> &'static str {
    if command.starts_with("deepcli ") {
        scorecard_checklist_label(command)
    } else if command.starts_with("cd ") && command.contains(" && deepcli resume ") {
        "Resume forked context"
    } else if command == "cargo test mvp_slash_commands_are_registered" {
        "Verify command registry"
    } else if command.starts_with("cargo ") {
        "Run cargo command"
    } else if command.starts_with("git config user.") {
        "Configure Git identity"
    } else if command.starts_with("git ") {
        "Run git command"
    } else if command.starts_with("mkdir ") || command.starts_with("ln ") {
        "Install shell command"
    } else if command.starts_with("chmod ") {
        "Update shell permissions"
    } else if command.starts_with("rm ") {
        "Remove stale shell file"
    } else if command.starts_with("cd ") {
        "Enter workspace"
    } else {
        "Run command"
    }
}

fn scorecard_checklist_label(command: &str) -> &'static str {
    match command {
        "deepcli quickstart" => "Open quickstart guide",
        "deepcli quickstart --check" => "Check quickstart readiness",
        "deepcli quickstart --json" => "Open quickstart readiness",
        "deepcli init --quick" => "Initialize project config",
        "deepcli config show --json" => "Inspect project config",
        "deepcli config sources --json" => "Inspect config sources",
        "deepcli config validate" => "Validate project config",
        "deepcli config validate --json" => "Validate project config",
        command if command.starts_with("deepcli config get ") => "Inspect config value",
        "deepcli recipes" => "Open workflow recipes",
        "deepcli recipes release" => "Open release workflow",
        "deepcli completion json" => "Export command catalog",
        command if command.starts_with("deepcli completion install ") => "Install shell completion",
        command if command.starts_with("deepcli completion status ") => "Check shell completion",
        "deepcli status --json" => "Inspect current status",
        command if command.starts_with("deepcli usage ") => "Inspect session usage",
        command if command.starts_with("deepcli trace ") => "Inspect session trace",
        command if command.starts_with("deepcli next ") => "Inspect recovery actions",
        command if command.starts_with("deepcli logs") => "Inspect local logs",
        "deepcli review" => "Review current diff",
        "deepcli test discover --json" => "Discover test commands",
        command if command.starts_with("deepcli test run ") => "Run test command",
        "deepcli help test" => "Open test help",
        "deepcli prompt list --json" => "List prompts",
        command if command.starts_with("deepcli prompt get ") => "Open prompt",
        command if command.starts_with("deepcli prompt render ") => "Render prompt",
        "deepcli help prompt" => "Open prompt help",
        "deepcli skill list --json" => "List skills",
        command if command.starts_with("deepcli skill run ") => "Run skill",
        "deepcli help skill" => "Open skill help",
        "deepcli agent list --json" => "List sub-agents",
        command if command.starts_with("deepcli agent show ") => "Inspect sub-agent",
        "deepcli help agent" => "Open agent help",
        "deepcli git status --json" => "Inspect git status",
        "deepcli git diff --json" => "Inspect git diff",
        "deepcli git message --json" => "Prepare commit message",
        "deepcli git branch --json" => "Inspect git branches",
        "deepcli help git" => "Open git help",
        "deepcli resume" => "Resume saved work",
        "deepcli resume --dry-run --json" => "Resume preview",
        "deepcli resume candidates --json" => "Inspect resume candidates",
        command if command.starts_with("deepcli resume ") && command.contains("--dry-run") => {
            "Resume preview"
        }
        command if command.starts_with("deepcli resume ") => "Resume saved work",
        "deepcli sessions --all --limit 20" => "List saved sessions",
        command if command.starts_with("deepcli history ") => "List saved sessions",
        "deepcli next --json" => "Inspect recovery actions",
        "deepcli handoff --pr" => "Prepare PR handoff",
        "deepcli permissions show --json" => "Inspect permissions",
        command if command.starts_with("deepcli permissions set-mode ") => "Set permission mode",
        "deepcli help permissions" => "Open permissions help",
        "deepcli credentials status --json" => "Inspect credentials",
        command if command.starts_with("deepcli credentials status ") => "Inspect credentials",
        command if command.starts_with("deepcli credentials set ") => {
            "Configure provider credentials"
        }
        "deepcli help credentials" => "Open credentials help",
        "deepcli model list" => "List configured models",
        "deepcli model list --json" => "List configured models",
        "deepcli model show --json" => "Inspect active model",
        "deepcli help model" => "Open model help",
        command if command.starts_with("deepcli model set ") => "Switch configured model",
        "deepcli timeout --json" => "Inspect provider timeout",
        "deepcli timeout reset" => "Reset provider timeout",
        "deepcli help timeout" => "Open timeout help",
        "deepcli stop" => "Stop running task",
        "deepcli fork --dry-run --json" => "Preview session fork",
        command if command.starts_with("deepcli fork --current") => "Fork active context",
        command if command.starts_with("deepcli fork ") => "Create session fork",
        "deepcli use deepseek deepseek-v4-pro" => "Switch to DeepSeek v4-pro",
        "deepcli doctor --quick" => "Run quick diagnostics",
        "deepcli doctor --quick --json" => "Run quick diagnostics",
        "deepcli doctor shell --json" => "Check shell install",
        "deepcli doctor docker --json" => "Check Docker environment",
        command if command.starts_with("deepcli doctor ") => "Check local environment",
        command if command.starts_with("deepcli compiler plan ") => "Inspect environment plan",
        command if command.starts_with("deepcli compiler test ") => "Run environment test",
        command if command.starts_with("deepcli install ") => "Set up local environment",
        "deepcli diagnose --json" => "Collect diagnostics",
        "deepcli diagnose --full-env --json" => "Run full diagnostics",
        "deepcli diagnose --probe-provider --json" => "Probe provider diagnostics",
        command if command.starts_with("deepcli diagnose --full-env --bundle ") => {
            "Create full support bundle"
        }
        "deepcli session diagnose --json" => "Inspect session diagnostics",
        command if command.starts_with("deepcli session diagnose ") => {
            "Inspect session diagnostics"
        }
        command if command.starts_with("deepcli session next ") => "Inspect recovery actions",
        command if command.starts_with("deepcli session list") => "List saved sessions",
        command
            if command.starts_with("deepcli session prune-empty ")
                && command.contains("--force") =>
        {
            "Delete empty sessions"
        }
        command if command.starts_with("deepcli session prune-empty ") => {
            "Preview empty session cleanup"
        }
        command if command.starts_with("deepcli session tools --failed") => "Inspect failed tools",
        command if command.starts_with("deepcli session tests ") => "Inspect session tests",
        command if command.starts_with("deepcli session history ") => "Inspect session history",
        command if command.starts_with("deepcli session summary ") => "Inspect session summary",
        "deepcli help session" => "Open session help",
        "deepcli help resume" => "Open resume help",
        command if command.starts_with("deepcli approval approve ") => "Approve request",
        command if command.starts_with("deepcli approval deny ") => "Deny request",
        command if command.starts_with("deepcli approval list ") => "Review approvals",
        "deepcli help approval" => "Open approval help",
        command if command.starts_with("deepcli btw list ") => "Review by-the-way questions",
        "deepcli help btw" => "Open by-the-way help",
        command if command.starts_with("deepcli support ") || command == "deepcli support" => {
            "Create support bundle"
        }
        "deepcli version --json" => "Inspect version",
        "deepcli scorecard --json" => "Inspect product scorecard",
        "deepcli recipes sota --json" => "Open SOTA product loop recipe",
        "deepcli opportunities" | "deepcli opportunities --json" => "Open product opportunities",
        command if command.starts_with("deepcli opportunities ") => "Open product opportunities",
        "deepcli benchmark presets --json" => "List benchmark presets",
        "deepcli benchmark status --json" => "Check benchmark evidence",
        "deepcli benchmark run-suite --json --fail-on-command" => "Run benchmark suite",
        "deepcli benchmark run --preset cargo-test --json --fail-on-command" => {
            "Run cargo-test benchmark"
        }
        "deepcli benchmark gate --json" => "Gate benchmark evidence",
        "deepcli benchmark summary --json" => "Review benchmark summary",
        "deepcli benchmark trends --json" => "Check benchmark trends",
        "deepcli benchmark baselines --json" => "List benchmark baselines",
        DEFAULT_BENCHMARK_CURRENT_BASELINE_TEMPLATE_ACTION => "Capture current benchmark baseline",
        DEFAULT_BENCHMARK_BASELINE_TEMPLATE_ACTION => "Create competitor baseline template",
        DEFAULT_BENCHMARK_BASELINE_COMPARE_ACTION => "Compare against competitor baseline",
        "deepcli round --json --run-benchmark --fail-on-command" => "Refresh benchmark evidence",
        "deepcli round --json" => "Review current product round",
        "deepcli accept --json" => "Run acceptance checks",
        command if command.starts_with("deepcli accept ") => "Run acceptance checks",
        command if command.starts_with("deepcli gate ") => "Run delivery gate",
        _ => generic_recipe_command_label(command),
    }
}
