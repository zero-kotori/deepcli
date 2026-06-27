use super::{
    required_arg, session_metadata_json, set_command_output_path, write_command_output, CommandExit,
};
use crate::privacy::redact_sensitive_text;
use crate::session::{
    GoalContract, GoalStatus, Plan, PlanStep, PlanStepStatus, Session, SessionStore,
};
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoalOptions {
    mode: GoalMode,
    objective: Option<String>,
    acceptance_commands: Vec<String>,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoalMode {
    Show,
    Start,
    Clear,
    Status,
    Gate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GoalSessionSource {
    Current,
    LatestWithGoal,
}

impl GoalSessionSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            GoalSessionSource::Current => "current",
            GoalSessionSource::LatestWithGoal => "latest_with_goal",
        }
    }
}

pub(crate) struct GoalSessionSelection {
    pub(crate) session: Session,
    pub(crate) goal: GoalContract,
    pub(crate) source: GoalSessionSource,
}

pub(crate) fn handle_goal(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_goal_options(&args)?;
    let explicit_show = args.iter().any(|arg| arg == "show");
    let store = SessionStore::new(workspace);

    let (output, should_fail) = match options.mode {
        GoalMode::Show => {
            if let Some(session_id) = current.as_deref() {
                let session = store.load(session_id)?;
                if let Some(goal) = session.load_goal()? {
                    let report =
                        format_goal_text_with_source(&goal, &session, GoalSessionSource::Current);
                    if options.json_output {
                        (
                            format_goal_json(
                                workspace,
                                &session,
                                &goal,
                                "show",
                                GoalSessionSource::Current,
                                &report,
                            )?,
                            false,
                        )
                    } else {
                        (report, false)
                    }
                } else if !explicit_show {
                    let goal = default_goal_contract(options.acceptance_commands);
                    session.save_goal(&goal)?;
                    session.save_plan(&goal_contract_plan(&goal))?;
                    let report =
                        format_goal_text_with_source(&goal, &session, GoalSessionSource::Current);
                    if options.json_output {
                        (
                            format_goal_json(
                                workspace,
                                &session,
                                &goal,
                                "created",
                                GoalSessionSource::Current,
                                &report,
                            )?,
                            false,
                        )
                    } else {
                        (report, false)
                    }
                } else if let Some(selection) = latest_session_with_goal(&store, Some(session_id))?
                {
                    let report = format_goal_text_with_source(
                        &selection.goal,
                        &selection.session,
                        selection.source,
                    );
                    if options.json_output {
                        (
                            format_goal_json(
                                workspace,
                                &selection.session,
                                &selection.goal,
                                "show",
                                selection.source,
                                &report,
                            )?,
                            false,
                        )
                    } else {
                        (report, false)
                    }
                } else {
                    ("no active goal".to_string(), false)
                }
            } else if explicit_show {
                if let Some(selection) = latest_session_with_goal(&store, None)? {
                    let report = format_goal_text_with_source(
                        &selection.goal,
                        &selection.session,
                        selection.source,
                    );
                    if options.json_output {
                        (
                            format_goal_json(
                                workspace,
                                &selection.session,
                                &selection.goal,
                                "show",
                                selection.source,
                                &report,
                            )?,
                            false,
                        )
                    } else {
                        (report, false)
                    }
                } else {
                    ("no active goal".to_string(), false)
                }
            } else {
                return Err(anyhow::anyhow!(
                    "`/goal` requires an active session when creating a goal; start deepcli or pass a task first"
                ));
            }
        }
        GoalMode::Start => {
            let session_id = current.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "`/goal start` requires an active session; start deepcli or pass a task first"
                )
            })?;
            let session = store.load(session_id)?;
            let mut goal = default_goal_contract(options.acceptance_commands);
            if let Some(objective) = options.objective {
                goal.objective = objective;
            }
            session.save_goal(&goal)?;
            session.save_plan(&goal_contract_plan(&goal))?;
            session.append_audit_event(
                "goal_started",
                json!({
                    "objective": redact_sensitive_text(&goal.objective),
                    "acceptance_commands": goal.acceptance_commands,
                }),
            )?;
            let report = format_goal_text_with_source(&goal, &session, GoalSessionSource::Current);
            if options.json_output {
                (
                    format_goal_json(
                        workspace,
                        &session,
                        &goal,
                        "created",
                        GoalSessionSource::Current,
                        &report,
                    )?,
                    false,
                )
            } else {
                (report, false)
            }
        }
        GoalMode::Clear => {
            let session_id = current.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "`/goal clear` requires an active session; use `/goal show` to inspect older goals"
                )
            })?;
            let session = store.load(session_id)?;
            let mut goal = session
                .load_goal()?
                .ok_or_else(|| anyhow::anyhow!("no active goal to clear"))?;
            goal.status = GoalStatus::Cancelled;
            goal.updated_at = Utc::now();
            session.save_goal(&goal)?;
            session.append_audit_event("goal_cancelled", json!({}))?;
            let report = "cancelled active goal".to_string();
            if options.json_output {
                (
                    format_goal_json(
                        workspace,
                        &session,
                        &goal,
                        "cancelled",
                        GoalSessionSource::Current,
                        &report,
                    )?,
                    false,
                )
            } else {
                (report, false)
            }
        }
        GoalMode::Status | GoalMode::Gate => {
            let selection = select_goal_session(&store, current.as_deref())?
                .ok_or_else(|| anyhow::anyhow!("no active goal"))?;
            let report = collect_goal_readiness(workspace, &selection.session, &selection.goal)?;
            let output = if options.json_output {
                format_goal_status_json(
                    workspace,
                    &selection.session,
                    &selection.goal,
                    selection.source,
                    &report,
                )?
            } else {
                format_goal_status_text_with_source(
                    report.report.clone(),
                    &selection.session,
                    selection.source,
                )
            };
            (
                output,
                matches!(options.mode, GoalMode::Gate) && !report.ready,
            )
        }
    };

    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    if should_fail {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

pub(crate) fn select_goal_session(
    store: &SessionStore,
    current: Option<&str>,
) -> Result<Option<GoalSessionSelection>> {
    if let Some(session_id) = current {
        let session = store.load(session_id)?;
        if let Some(goal) = session.load_goal()? {
            return Ok(Some(GoalSessionSelection {
                session,
                goal,
                source: GoalSessionSource::Current,
            }));
        }
        return latest_session_with_goal(store, Some(session_id));
    }
    latest_session_with_goal(store, None)
}

fn latest_session_with_goal(
    store: &SessionStore,
    skip_id: Option<&str>,
) -> Result<Option<GoalSessionSelection>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        if let Some(goal) = session.load_goal()? {
            return Ok(Some(GoalSessionSelection {
                session,
                goal,
                source: GoalSessionSource::LatestWithGoal,
            }));
        }
    }
    Ok(None)
}

fn parse_goal_options(args: &[String]) -> Result<GoalOptions> {
    let mut mode = GoalMode::Show;
    let mut objective_parts = Vec::new();
    let mut acceptance_commands = Vec::new();
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "show" if objective_parts.is_empty() && mode == GoalMode::Show => {
                mode = GoalMode::Show;
                index += 1;
            }
            "start" if objective_parts.is_empty() && mode == GoalMode::Show => {
                mode = GoalMode::Start;
                index += 1;
            }
            "status" | "check" if objective_parts.is_empty() && mode == GoalMode::Show => {
                mode = GoalMode::Status;
                index += 1;
            }
            "gate" if objective_parts.is_empty() && mode == GoalMode::Show => {
                mode = GoalMode::Gate;
                index += 1;
            }
            "clear" | "cancel" if objective_parts.is_empty() && mode == GoalMode::Show => {
                mode = GoalMode::Clear;
                index += 1;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            "--acceptance-cmd" | "--acceptance-command" => {
                let raw = required_arg(args, index + 1, "acceptance command")?;
                acceptance_commands.push(raw.trim().to_string());
                index += 2;
            }
            value if value.starts_with("--acceptance-cmd=") => {
                let raw = value.trim_start_matches("--acceptance-cmd=").trim();
                if raw.is_empty() {
                    bail!("--acceptance-cmd requires a command");
                }
                acceptance_commands.push(raw.to_string());
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /goal option `{value}`"),
            value => {
                if matches!(mode, GoalMode::Status | GoalMode::Gate | GoalMode::Clear) {
                    bail!("unexpected /goal argument `{value}`");
                }
                if mode == GoalMode::Show {
                    mode = GoalMode::Start;
                }
                objective_parts.push(value.to_string());
                index += 1;
            }
        }
    }
    acceptance_commands.retain(|command| !command.trim().is_empty());
    let objective = (!objective_parts.is_empty()).then(|| objective_parts.join(" "));
    Ok(GoalOptions {
        mode,
        objective,
        acceptance_commands,
        json_output,
        output_path,
    })
}

fn default_goal_contract(extra_acceptance_commands: Vec<String>) -> GoalContract {
    let now = Utc::now();
    let mut acceptance_commands = vec![
        "cargo fmt --check".to_string(),
        "cargo clippy --all-targets -- -D warnings".to_string(),
        "cargo test".to_string(),
        "./scripts/deepcli preflight --json".to_string(),
        "./scripts/deepcli round --json --run-benchmark --fail-on-command".to_string(),
    ];
    acceptance_commands.extend(extra_acceptance_commands);
    GoalContract {
        objective: "完整实现当前项目文档中的全部需求，并且只有在所有验收要求通过、所有测试通过且目标达成后才可停止。".to_string(),
        source_requirements: vec![
            "README.md".to_string(),
            "docs/FEATURES.md".to_string(),
            "docs/ai/REQUIREMENTS.md".to_string(),
            "docs/ai/TECHNICAL_PLAN.md".to_string(),
            "docs/ai/CONTEXT.md".to_string(),
        ],
        stop_conditions: vec![
            "已经逐项检查当前项目文档中的明确需求、命令、门禁和验收要求。".to_string(),
            "实现覆盖所有未完成需求，且没有用更窄范围替代原目标。".to_string(),
            "所有相关测试、格式检查、lint、preflight 和产品 round/benchmark 门禁均通过。".to_string(),
            "隐私、命名和提交身份扫描未发现阻断问题。".to_string(),
            "工作区状态、残余风险和未验证项已明确汇报；若存在未验证项，不得宣称 goal 完成。".to_string(),
        ],
        acceptance_commands,
        status: GoalStatus::Active,
        created_at: now,
        updated_at: now,
    }
}

fn goal_contract_plan(goal: &GoalContract) -> Plan {
    Plan {
        title: "Goal contract: complete documented project requirements".to_string(),
        updated_at: Utc::now(),
        steps: vec![
            PlanStep {
                id: "goal_context".to_string(),
                description: format!(
                    "Read and derive requirements from: {}.",
                    goal.source_requirements.join(", ")
                ),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "goal_implementation".to_string(),
                description:
                    "Implement the missing product behavior without narrowing the goal scope."
                        .to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "goal_tests".to_string(),
                description: "Run all acceptance commands and repair any failure.".to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "goal_privacy".to_string(),
                description: "Run privacy, naming, artifact, and Git identity checks.".to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                id: "goal_completion_audit".to_string(),
                description:
                    "Prove every requirement is satisfied before claiming the goal is complete."
                        .to_string(),
                status: PlanStepStatus::Pending,
            },
        ],
    }
}

fn format_goal_text(goal: &GoalContract) -> String {
    let mut lines = vec![
        "active goal contract".to_string(),
        format!("status: {:?}", goal.status),
        format!("objective: {}", redact_sensitive_text(&goal.objective)),
        "requirement sources:".to_string(),
    ];
    lines.extend(
        goal.source_requirements
            .iter()
            .map(|item| format!("  - {}", redact_sensitive_text(item))),
    );
    lines.push("stop conditions:".to_string());
    lines.extend(
        goal.stop_conditions
            .iter()
            .map(|item| format!("  - {}", redact_sensitive_text(item))),
    );
    lines.push("acceptance commands:".to_string());
    lines.extend(
        goal.acceptance_commands
            .iter()
            .map(|item| format!("  - {}", redact_sensitive_text(item))),
    );
    lines.push(
        "next: continue implementation; do not claim completion until all conditions pass"
            .to_string(),
    );
    lines.join("\n")
}

fn format_goal_text_with_source(
    goal: &GoalContract,
    session: &Session,
    source: GoalSessionSource,
) -> String {
    format!(
        "session source: {}\nsession: {}\n{}",
        source.as_str(),
        session.id(),
        format_goal_text(goal)
    )
}

fn format_goal_json(
    workspace: &Path,
    session: &Session,
    goal: &GoalContract,
    status: &str,
    source: GoalSessionSource,
    report: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.goal.v1",
        "status": status,
        "workspace": workspace.display().to_string(),
        "sessionSource": source.as_str(),
        "session": session_metadata_json(&session.metadata),
        "goal": goal,
        "report": report,
    }))?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoalReadinessReport {
    pub(crate) ready: bool,
    pub(crate) blockers: Vec<String>,
    missing_sources: Vec<String>,
    pub(crate) plan: GoalPlanReadiness,
    pub(crate) acceptance: Vec<GoalAcceptanceEvidence>,
    pub(crate) report: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoalPlanReadiness {
    pub(crate) present: bool,
    pub(crate) total: usize,
    pub(crate) completed: usize,
    pub(crate) pending: usize,
    pub(crate) in_progress: usize,
    pub(crate) failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoalAcceptanceEvidence {
    pub(crate) command: String,
    pub(crate) status: &'static str,
    pub(crate) exit_code: Option<i32>,
    pub(crate) created_at: Option<DateTime<Utc>>,
}

pub(crate) fn collect_goal_readiness(
    workspace: &Path,
    session: &Session,
    goal: &GoalContract,
) -> Result<GoalReadinessReport> {
    let mut blockers = Vec::new();
    if !matches!(goal.status, GoalStatus::Active | GoalStatus::Complete) {
        blockers.push(format!("goal status is {:?}, not active", goal.status));
    }

    let missing_sources = goal
        .source_requirements
        .iter()
        .filter(|source| !workspace.join(source.as_str()).exists())
        .cloned()
        .collect::<Vec<_>>();
    if !missing_sources.is_empty() {
        blockers.push(format!(
            "missing requirement source(s): {}",
            missing_sources.join(", ")
        ));
    }

    let plan = goal_plan_readiness(session)?;
    if !plan.present {
        blockers.push("goal guard plan is missing".to_string());
    } else if plan.pending + plan.in_progress + plan.failed > 0 {
        blockers.push(format!(
            "plan has {} pending, {} in progress, and {} failed step(s)",
            plan.pending, plan.in_progress, plan.failed
        ));
    }

    if goal.acceptance_commands.is_empty() {
        blockers.push("goal has no acceptance commands".to_string());
    }
    let acceptance = goal_acceptance_evidence(session, &goal.acceptance_commands)?;
    for item in &acceptance {
        match item.status {
            "missing" => blockers.push(format!(
                "missing passing evidence for acceptance command `{}`; run `/test run -- {}`",
                redact_sensitive_text(&item.command),
                redact_sensitive_text(&item.command)
            )),
            "failed" => blockers.push(format!(
                "latest evidence for acceptance command `{}` failed",
                redact_sensitive_text(&item.command)
            )),
            _ => {}
        }
    }

    let ready = blockers.is_empty();
    let report =
        format_goal_status_text(goal, ready, &blockers, &missing_sources, &plan, &acceptance);
    Ok(GoalReadinessReport {
        ready,
        blockers,
        missing_sources,
        plan,
        acceptance,
        report,
    })
}

fn goal_plan_readiness(session: &Session) -> Result<GoalPlanReadiness> {
    let Some(plan) = session.load_plan()? else {
        return Ok(GoalPlanReadiness {
            present: false,
            total: 0,
            completed: 0,
            pending: 0,
            in_progress: 0,
            failed: 0,
        });
    };
    let mut readiness = GoalPlanReadiness {
        present: true,
        total: plan.steps.len(),
        completed: 0,
        pending: 0,
        in_progress: 0,
        failed: 0,
    };
    for step in plan.steps {
        match step.status {
            PlanStepStatus::Pending => readiness.pending += 1,
            PlanStepStatus::InProgress => readiness.in_progress += 1,
            PlanStepStatus::Completed => readiness.completed += 1,
            PlanStepStatus::Failed => readiness.failed += 1,
        }
    }
    Ok(readiness)
}

fn goal_acceptance_evidence(
    session: &Session,
    commands: &[String],
) -> Result<Vec<GoalAcceptanceEvidence>> {
    let tests = session.load_test_runs()?;
    Ok(commands
        .iter()
        .map(|command| {
            let latest = tests
                .iter()
                .filter(|test| test.command == *command)
                .max_by_key(|test| test.created_at);
            match latest {
                Some(test) if test.passed => GoalAcceptanceEvidence {
                    command: command.clone(),
                    status: "passed",
                    exit_code: test.exit_code,
                    created_at: Some(test.created_at),
                },
                Some(test) => GoalAcceptanceEvidence {
                    command: command.clone(),
                    status: "failed",
                    exit_code: test.exit_code,
                    created_at: Some(test.created_at),
                },
                None => GoalAcceptanceEvidence {
                    command: command.clone(),
                    status: "missing",
                    exit_code: None,
                    created_at: None,
                },
            }
        })
        .collect())
}

fn format_goal_status_text(
    goal: &GoalContract,
    ready: bool,
    blockers: &[String],
    missing_sources: &[String],
    plan: &GoalPlanReadiness,
    acceptance: &[GoalAcceptanceEvidence],
) -> String {
    let mut lines = vec![
        "goal readiness".to_string(),
        format!("ready: {ready}"),
        format!("goal status: {:?}", goal.status),
        format!("objective: {}", redact_sensitive_text(&goal.objective)),
        format!(
            "plan: present={} total={} completed={} pending={} in_progress={} failed={}",
            plan.present, plan.total, plan.completed, plan.pending, plan.in_progress, plan.failed
        ),
    ];
    if missing_sources.is_empty() {
        lines.push("requirement sources: ok".to_string());
    } else {
        lines.push(format!(
            "missing requirement sources: {}",
            missing_sources.join(", ")
        ));
    }
    lines.push("acceptance evidence:".to_string());
    lines.extend(acceptance.iter().map(|item| {
        let when = item
            .created_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "<none>".to_string());
        format!(
            "  - [{}] {} exit={:?} at={}",
            item.status,
            redact_sensitive_text(&item.command),
            item.exit_code,
            when
        )
    }));
    if blockers.is_empty() {
        lines.push("blockers: none".to_string());
        lines.push("next: the goal is ready for final human review or completion".to_string());
    } else {
        lines.push("blockers:".to_string());
        lines.extend(
            blockers
                .iter()
                .map(|item| format!("  - {}", redact_sensitive_text(item))),
        );
        lines.push(
            "next: keep working; `/goal gate` will fail until blockers are resolved".to_string(),
        );
    }
    lines.join("\n")
}

fn format_goal_status_text_with_source(
    report: String,
    session: &Session,
    source: GoalSessionSource,
) -> String {
    format!(
        "session source: {}\nsession: {}\n{}",
        source.as_str(),
        session.id(),
        report
    )
}

fn format_goal_status_json(
    workspace: &Path,
    session: &Session,
    goal: &GoalContract,
    source: GoalSessionSource,
    report: &GoalReadinessReport,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.goal.status.v1",
        "status": if report.ready { "ready" } else { "blocked" },
        "ready": report.ready,
        "workspace": workspace.display().to_string(),
        "sessionSource": source.as_str(),
        "session": session_metadata_json(&session.metadata),
        "goal": goal,
        "blockers": report.blockers,
        "missingSources": report.missing_sources,
        "plan": {
            "present": report.plan.present,
            "total": report.plan.total,
            "completed": report.plan.completed,
            "pending": report.plan.pending,
            "inProgress": report.plan.in_progress,
            "failed": report.plan.failed,
        },
        "acceptance": report.acceptance.iter().map(|item| json!({
            "command": redact_sensitive_text(&item.command),
            "status": item.status,
            "exitCode": item.exit_code,
            "createdAt": item.created_at,
        })).collect::<Vec<_>>(),
        "nextActions": if report.ready {
            vec!["perform final human review before marking the broader goal complete"]
        } else {
            vec![
                "complete pending goal plan steps",
                "run each acceptance command through `/test run -- <command>`",
                "rerun `/goal gate --json`"
            ]
        },
        "report": report.report,
    }))?)
}
