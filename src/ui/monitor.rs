use crate::runtime::{
    SessionMonitor, SessionObservation, SessionObservationApproval, SessionObservationQuestion,
    SessionObservationTest,
};

use super::{
    compact_ui_text, format_cache_hit_rate, format_latest_environment, format_optional_bytes,
    format_optional_u64, short_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MonitorTier {
    Core,
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MonitorTab {
    Overview,
    Changes,
    Tools,
    Tests,
    Session,
    Approvals,
    Context,
    Result,
    Usage,
    Health,
    Library,
    Deliver,
    Environment,
    Trace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MonitorTabMetadata {
    pub(super) tab: MonitorTab,
    pub(super) label: &'static str,
    pub(super) tier: MonitorTier,
}

const MONITOR_TAB_METADATA: &[MonitorTabMetadata] = &[
    // core task views first
    monitor_tab(MonitorTab::Overview, "Overview", MonitorTier::Core),
    monitor_tab(MonitorTab::Changes, "Changes", MonitorTier::Core),
    monitor_tab(MonitorTab::Tools, "Tools", MonitorTier::Core),
    monitor_tab(MonitorTab::Tests, "Tests", MonitorTier::Core),
    monitor_tab(MonitorTab::Session, "Session", MonitorTier::Core),
    monitor_tab(MonitorTab::Approvals, "Approvals", MonitorTier::Core),
    monitor_tab(MonitorTab::Context, "Context", MonitorTier::Core),
    // advanced / support diagnostics
    monitor_tab(MonitorTab::Result, "Result", MonitorTier::Advanced),
    monitor_tab(MonitorTab::Usage, "Usage", MonitorTier::Advanced),
    monitor_tab(MonitorTab::Health, "Health", MonitorTier::Advanced),
    monitor_tab(MonitorTab::Library, "Library", MonitorTier::Advanced),
    monitor_tab(MonitorTab::Deliver, "Deliver", MonitorTier::Advanced),
    monitor_tab(
        MonitorTab::Environment,
        "Environment",
        MonitorTier::Advanced,
    ),
    monitor_tab(MonitorTab::Trace, "Trace", MonitorTier::Advanced),
];

const fn monitor_tab(
    tab: MonitorTab,
    label: &'static str,
    tier: MonitorTier,
) -> MonitorTabMetadata {
    MonitorTabMetadata { tab, label, tier }
}

impl MonitorTab {
    pub(super) fn all() -> Vec<Self> {
        Self::metadata().iter().map(|entry| entry.tab).collect()
    }

    pub(super) fn metadata() -> &'static [MonitorTabMetadata] {
        MONITOR_TAB_METADATA
    }

    pub(super) fn metadata_for(self) -> &'static MonitorTabMetadata {
        Self::metadata()
            .iter()
            .find(|entry| entry.tab == self)
            .unwrap_or_else(|| panic!("monitor tab metadata missing for {self:?}"))
    }

    pub(super) fn static_quick_action_metadata() -> &'static [MonitorTabQuickActions] {
        MONITOR_STATIC_QUICK_ACTIONS
    }

    pub(super) fn static_quick_actions(self) -> Option<&'static MonitorTabQuickActions> {
        Self::static_quick_action_metadata()
            .iter()
            .find(|entry| entry.tab == self)
    }

    pub(super) fn running_quick_actions(self) -> Option<&'static MonitorTabQuickActions> {
        MONITOR_RUNNING_QUICK_ACTIONS
            .iter()
            .find(|entry| entry.tab == self)
    }

    pub(super) fn tier(self) -> MonitorTier {
        self.metadata_for().tier
    }

    pub(super) fn next(self) -> Self {
        let tabs = Self::all();
        let idx = tabs.iter().position(|tab| *tab == self).unwrap_or(0);
        tabs[(idx + 1) % tabs.len()]
    }

    pub(super) fn previous(self) -> Self {
        let tabs = Self::all();
        let idx = tabs.iter().position(|tab| *tab == self).unwrap_or(0);
        tabs[(idx + tabs.len() - 1) % tabs.len()]
    }

    pub(super) fn label(self) -> &'static str {
        self.metadata_for().label
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MonitorQuickAction {
    pub(super) command: String,
    pub(super) edit_before_run: bool,
}

impl MonitorQuickAction {
    pub(super) fn run(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            edit_before_run: false,
        }
    }

    pub(super) fn edit(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            edit_before_run: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MonitorQuickActionTemplate {
    command: &'static str,
    edit_before_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MonitorTabQuickActions {
    pub(super) tab: MonitorTab,
    actions: &'static [MonitorQuickActionTemplate],
}

const MONITOR_OVERVIEW_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/status --json"),
    monitor_run_action("/next --json"),
    monitor_run_action("/trace --limit 30"),
];

const MONITOR_RESULT_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/trace --limit 30"),
    monitor_run_action("/status --json"),
    monitor_run_action("/session history --limit 5"),
];

const MONITOR_CHANGES_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/diff --stat"),
    monitor_run_action("/diff --name-only"),
    monitor_run_action("/review"),
    monitor_run_action("/handoff --format pr"),
];

const MONITOR_USAGE_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/usage --json"),
    monitor_run_action("/trace --limit 30"),
    monitor_run_action("/logs --limit 80"),
    monitor_run_action("/status --json"),
];

const MONITOR_TESTS_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/test discover --json"),
    monitor_run_action("/test run --json"),
    monitor_run_action("/accept --json"),
    monitor_run_action("/gate --json"),
];

const MONITOR_SESSION_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/session diagnose --json"),
    monitor_run_action("/goal status --json"),
    monitor_run_action("/plan show --json"),
    monitor_run_action("/fork --dry-run --json"),
];

const MONITOR_APPROVALS_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[];

const MONITOR_CONTEXT_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/context"),
    monitor_run_action("/usage --json"),
    monitor_run_action("/status --json"),
    monitor_run_action("/trace --limit 30"),
];

const MONITOR_TRACE_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/trace --limit 30"),
    monitor_run_action("/logs --limit 80"),
    monitor_run_action("/usage --json"),
    monitor_run_action("/session diagnose --json"),
];

const MONITOR_OVERVIEW_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/status"),
    monitor_run_action("/usage --json"),
    monitor_run_action("/trace --limit 30"),
];

const MONITOR_RESULT_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/trace --limit 30"),
    monitor_run_action("/status"),
    monitor_run_action("/session history --limit 5"),
];

const MONITOR_CHANGES_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/git status --json"),
    monitor_run_action("/git diff --stat"),
];

const MONITOR_TESTS_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/session tests --limit 5"),
    monitor_run_action("/usage --json"),
];

const MONITOR_SESSION_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/session diagnose --json"),
    monitor_run_action("/session history --limit 10"),
    monitor_run_action("/fork --dry-run --json"),
];

const MONITOR_CONTEXT_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/usage --json"),
    monitor_run_action("/status"),
    monitor_run_action("/trace --limit 30"),
];

const MONITOR_TRACE_RUNNING_QUICK_ACTIONS: &[MonitorQuickActionTemplate] = &[
    monitor_run_action("/trace --limit 30"),
    monitor_run_action("/logs --limit 80"),
    monitor_run_action("/usage --json"),
    monitor_run_action("/session diagnose --json"),
];

const MONITOR_STATIC_QUICK_ACTIONS: &[MonitorTabQuickActions] = &[
    monitor_tab_quick_actions(MonitorTab::Overview, MONITOR_OVERVIEW_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Result, MONITOR_RESULT_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Changes, MONITOR_CHANGES_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Usage, MONITOR_USAGE_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Tests, MONITOR_TESTS_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Session, MONITOR_SESSION_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Approvals, MONITOR_APPROVALS_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Context, MONITOR_CONTEXT_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Trace, MONITOR_TRACE_QUICK_ACTIONS),
];

const MONITOR_RUNNING_QUICK_ACTIONS: &[MonitorTabQuickActions] = &[
    monitor_tab_quick_actions(MonitorTab::Overview, MONITOR_OVERVIEW_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Result, MONITOR_RESULT_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Changes, MONITOR_CHANGES_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Usage, MONITOR_CONTEXT_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Tests, MONITOR_TESTS_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Session, MONITOR_SESSION_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Context, MONITOR_CONTEXT_RUNNING_QUICK_ACTIONS),
    monitor_tab_quick_actions(MonitorTab::Trace, MONITOR_TRACE_RUNNING_QUICK_ACTIONS),
];

const fn monitor_run_action(command: &'static str) -> MonitorQuickActionTemplate {
    MonitorQuickActionTemplate {
        command,
        edit_before_run: false,
    }
}

const fn monitor_tab_quick_actions(
    tab: MonitorTab,
    actions: &'static [MonitorQuickActionTemplate],
) -> MonitorTabQuickActions {
    MonitorTabQuickActions { tab, actions }
}

impl MonitorQuickActionTemplate {
    fn to_action(self) -> MonitorQuickAction {
        if self.edit_before_run {
            MonitorQuickAction::edit(self.command)
        } else {
            MonitorQuickAction::run(self.command)
        }
    }
}

impl MonitorTabQuickActions {
    pub(super) fn actions(&self) -> Vec<MonitorQuickAction> {
        self.actions
            .iter()
            .map(|action| action.to_action())
            .collect()
    }
}

pub(super) fn append_monitor_quick_actions(
    lines: &mut Vec<String>,
    label: &str,
    actions: &[MonitorQuickAction],
    selected: usize,
) {
    if actions.is_empty() {
        return;
    }
    lines.push(format!(
        "{label} (Up/Down select, Enter {}):",
        quick_action_enter_label(actions)
    ));
    let selected = selected.min(actions.len() - 1);
    for (index, action) in actions.iter().enumerate() {
        let marker = if index == selected { ">" } else { " " };
        let suffix = if action.edit_before_run {
            " (edit)"
        } else {
            ""
        };
        lines.push(format!(" {marker} {}{suffix}", action.command));
    }
}

pub(super) fn tool_quick_actions() -> Vec<MonitorQuickAction> {
    vec![
        MonitorQuickAction::edit("/session tools --limit 20 --current"),
        MonitorQuickAction::edit("/session tools --failed --limit 20 --current"),
    ]
}

pub(super) fn environment_quick_actions(
    monitor: Option<&SessionMonitor>,
) -> Vec<MonitorQuickAction> {
    let target = environment_action_target(monitor);
    let mut actions = vec![MonitorQuickAction::run(format!("/doctor {target} --json"))];
    if target == "compiler" {
        actions.push(MonitorQuickAction::run("/compiler plan --smoke --json"));
    }
    if environment_needs_setup(monitor) {
        actions.push(MonitorQuickAction::edit(format!(
            "/install {target} --smoke"
        )));
    }
    if target == "compiler" {
        actions.push(MonitorQuickAction::run("/compiler test --json"));
    }
    actions.extend([
        MonitorQuickAction::run(format!("/accept --env-check {target} --json")),
        MonitorQuickAction::run(format!("/gate --env-check {target} --json")),
        MonitorQuickAction::run(format!("/handoff --env-check {target} --format pr")),
    ]);
    actions
}

pub(super) fn deliver_quick_actions(monitor: Option<&SessionMonitor>) -> Vec<MonitorQuickAction> {
    if monitor.is_some() {
        let target = environment_action_target(monitor);
        vec![
            MonitorQuickAction::run("/review"),
            MonitorQuickAction::run("/test run --json"),
            MonitorQuickAction::run(format!("/accept --env-check {target} --json")),
            MonitorQuickAction::run(format!("/gate --env-check {target} --json")),
            MonitorQuickAction::run(format!("/handoff --env-check {target} --format pr")),
        ]
    } else {
        vec![
            MonitorQuickAction::run("/test discover --json"),
            MonitorQuickAction::run("/accept --json"),
            MonitorQuickAction::run("/gate --json"),
            MonitorQuickAction::run("/handoff --format pr"),
        ]
    }
}

pub(super) fn environment_action_target(monitor: Option<&SessionMonitor>) -> String {
    monitor
        .and_then(|monitor| monitor.recent_environment.last())
        .map(|environment| environment.target.as_str())
        .filter(|target| matches!(*target, "docker" | "compiler"))
        .unwrap_or("docker")
        .to_string()
}

pub(super) fn environment_needs_setup(monitor: Option<&SessionMonitor>) -> bool {
    monitor
        .and_then(|monitor| monitor.recent_environment.last())
        .map(|environment| {
            environment.ready == Some(false)
                || environment.status.contains("needs")
                || environment.status.contains("missing")
                || environment.detail.contains("/install")
        })
        .unwrap_or(true)
}

fn quick_action_enter_label(actions: &[MonitorQuickAction]) -> &'static str {
    let has_edit = actions.iter().any(|action| action.edit_before_run);
    let has_run = actions.iter().any(|action| !action.edit_before_run);
    match (has_run, has_edit) {
        (true, true) => "run/edit",
        (false, true) => "edit",
        _ => "run",
    }
}

pub(super) fn format_session_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["session unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let observation = &monitor.observation;
    let plan = if observation.plan_total == 0 {
        "plan=none".to_string()
    } else {
        format!(
            "plan={}/{} running={} failed={}",
            observation.plan_completed,
            observation.plan_total,
            observation.plan_in_progress,
            observation.plan_failed
        )
    };
    let current = observation
        .current_step
        .as_deref()
        .map(|step| format!("current={}", compact_ui_text(step, 64)))
        .unwrap_or_else(|| "current=none".to_string());
    let mut lines = vec![
        format!("session: state={} {plan} {current}", observation.state),
        format!(
            "queues: approvals={} btw={} tools={} failed_tools={}",
            observation.pending_approvals,
            observation.open_questions,
            observation.tool_calls,
            observation.failed_tools
        ),
    ];
    if monitor.recent_events.is_empty() {
        lines.push("recent events: none".to_string());
    } else {
        lines.push("recent events:".to_string());
        lines.extend(monitor.recent_events.iter().rev().take(3).map(|event| {
            format!(
                "  {} {}",
                event.created_at,
                compact_ui_text(&event.event_type, 60)
            )
        }));
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn format_context_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["context unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let usage = &monitor.usage;
    let mut lines = vec![
        format!(
            "context cache: hit={} miss={} hit_rate={}",
            format_optional_u64(usage.prompt_cache_hit_tokens),
            format_optional_u64(usage.prompt_cache_miss_tokens),
            format_cache_hit_rate(usage)
        ),
        format!(
            "request: latest={} max={} compacted_turns={}",
            format_optional_bytes(usage.latest_request_bytes),
            format_optional_bytes(usage.max_request_bytes),
            usage.compacted_turns
        ),
        format!(
            "tokens: prompt={} completion={} total={}",
            format_optional_u64(usage.prompt_tokens),
            format_optional_u64(usage.completion_tokens),
            format_optional_u64(usage.total_tokens)
        ),
    ];
    if let Some(environment) = monitor.recent_environment.last() {
        lines.push(format!(
            "environment: {}",
            format_latest_environment(environment)
        ));
    } else {
        lines.push("environment: none".to_string());
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn format_usage_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["usage unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let usage = &monitor.usage;
    let mut lines = Vec::new();
    if usage.provider_turns_started == 0
        && usage.provider_turns_completed == 0
        && usage.total_tokens.is_none()
        && usage.max_request_bytes.is_none()
    {
        lines.push("no provider usage recorded yet".to_string());
    } else {
        lines.push(format!(
            "provider turns: started={} completed={} avg={} max={} tool_calls={}",
            usage.provider_turns_started,
            usage.provider_turns_completed,
            format_optional_ms(usage.provider_average_elapsed_ms),
            format_optional_ms(usage.provider_max_elapsed_ms),
            usage.provider_tool_calls
        ));
        lines.push(format!(
            "tokens: prompt={} completion={} total={}",
            format_optional_u64(usage.prompt_tokens),
            format_optional_u64(usage.completion_tokens),
            format_optional_u64(usage.total_tokens)
        ));
        lines.push(format!(
            "request: latest={} max={} compacted_turns={}",
            format_optional_bytes(usage.latest_request_bytes),
            format_optional_bytes(usage.max_request_bytes),
            usage.compacted_turns
        ));
        if usage.prompt_cache_hit_tokens.is_some() || usage.prompt_cache_miss_tokens.is_some() {
            lines.push(format!(
                "context cache: hit={} miss={} hit_rate={}",
                format_optional_u64(usage.prompt_cache_hit_tokens),
                format_optional_u64(usage.prompt_cache_miss_tokens),
                format_cache_hit_rate(usage)
            ));
        }
        if usage.provider_turns_started > usage.provider_turns_completed {
            lines.push(
                "warning: provider turn started but not completed; inspect /trace --limit 30"
                    .to_string(),
            );
        }
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn format_deliver_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["delivery evidence unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let observation = &monitor.observation;
    let mut lines = vec!["acceptance checklist:".to_string()];
    lines.push(format!("  plan: {}", delivery_plan_status(observation)));
    lines.push(format!("  tests: {}", delivery_test_status(monitor)));
    lines.push(format!(
        "  environment: {}",
        delivery_environment_status(monitor)
    ));
    lines.push(format!(
        "  blockers: approvals={} btw={} failed_tools={}",
        observation.pending_approvals, observation.open_questions, observation.failed_tools
    ));
    append_monitor_quick_actions(
        &mut lines,
        "recommended flow",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn format_tests_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        let mut lines = vec!["tests unavailable for running handoff".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let mut lines = Vec::new();
    if monitor.recent_tests.is_empty() {
        lines.push("no test runs recorded".to_string());
    } else {
        lines.extend(monitor.recent_tests.iter().rev().map(format_latest_test));
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn format_environment_tab_lines(
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    match monitor {
        Some(monitor) if monitor.recent_environment.is_empty() => {
            lines.push("no environment evidence recorded".to_string());
        }
        Some(monitor) => {
            lines.push("recent environment evidence:".to_string());
            lines.extend(
                monitor
                    .recent_environment
                    .iter()
                    .rev()
                    .map(format_latest_environment),
            );
        }
        None => lines.push("environment evidence unavailable for running handoff".to_string()),
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn format_approvals_tab_lines(
    monitor: Option<&SessionMonitor>,
    selected_approval: usize,
) -> Vec<String> {
    let Some(monitor) = monitor else {
        return vec!["approvals unavailable for running handoff".to_string()];
    };
    let mut lines = Vec::new();
    if monitor.pending_approvals.is_empty() {
        lines.push("pending approvals: none".to_string());
    } else {
        lines.push(format!(
            "pending approvals: {} (Up/Down select, Enter approve, d deny)",
            monitor.pending_approvals.len()
        ));
        lines.extend(
            monitor
                .pending_approvals
                .iter()
                .enumerate()
                .map(|(index, approval)| {
                    format_pending_approval(index == selected_approval, approval)
                }),
        );
    }
    if monitor.open_questions.is_empty() {
        lines.push("open btw questions: none".to_string());
    } else {
        lines.push(format!(
            "open btw questions: {} (Enter opens answer box)",
            monitor.open_questions.len()
        ));
        let approval_count = monitor.pending_approvals.len();
        lines.extend(
            monitor
                .open_questions
                .iter()
                .enumerate()
                .map(|(index, question)| {
                    format_open_question(approval_count + index == selected_approval, question)
                }),
        );
    }
    lines
}

fn format_pending_approval(selected: bool, approval: &SessionObservationApproval) -> String {
    let marker = if selected { "*" } else { "-" };
    format!(
        "{marker} {} {} risk={} {}",
        short_id(&approval.id),
        approval.tool,
        approval.risk,
        compact_ui_text(&approval.reason, 70)
    )
}

fn format_open_question(selected: bool, question: &SessionObservationQuestion) -> String {
    let marker = if selected { "*" } else { "-" };
    format!(
        "{marker} {} {}",
        short_id(&question.id),
        compact_ui_text(&question.question, 82)
    )
}

pub(super) fn format_latest_test(test: &SessionObservationTest) -> String {
    let status = if test.passed { "pass" } else { "fail" };
    let code = test
        .exit_code
        .map(|code| format!(" code={code}"))
        .unwrap_or_default();
    format!(
        "test={}{} {}",
        status,
        code,
        compact_ui_text(&test.command, 42)
    )
}

fn delivery_plan_status(observation: &SessionObservation) -> String {
    if observation.plan_total == 0 {
        return "missing plan; run /plan".to_string();
    }
    if observation.plan_failed > 0 {
        return format!(
            "blocked failed={}/{}",
            observation.plan_failed, observation.plan_total
        );
    }
    if observation.plan_completed == observation.plan_total {
        return format!(
            "ok {}/{}",
            observation.plan_completed, observation.plan_total
        );
    }
    format!(
        "pending {}/{} running={}",
        observation.plan_completed, observation.plan_total, observation.plan_in_progress
    )
}

fn delivery_test_status(monitor: &SessionMonitor) -> String {
    let latest = monitor
        .observation
        .latest_test
        .as_ref()
        .or_else(|| monitor.recent_tests.last());
    match latest {
        Some(test) if test.passed => format!("ok {}", compact_ui_text(&test.command, 50)),
        Some(test) => format!(
            "failing code={} {}",
            test.exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "-".to_string()),
            compact_ui_text(&test.command, 50)
        ),
        None => "missing; run /test run --json".to_string(),
    }
}

fn delivery_environment_status(monitor: &SessionMonitor) -> String {
    match monitor.recent_environment.last() {
        Some(environment) if environment.ready == Some(true) => {
            format!("ok target={}", environment.target)
        }
        Some(environment) => format!(
            "{} target={}",
            environment.status,
            compact_ui_text(&environment.target, 32)
        ),
        None => "not requested; add --env-check when Docker/compiler matters".to_string(),
    }
}

fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(|value| format!("{value}ms"))
        .unwrap_or_else(|| "-".to_string())
}
