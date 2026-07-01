use super::{
    compact_ui_text, format_action_event, short_id, stop_running_task, ActiveSessionRef, ChatLine,
    TuiState,
};
use crate::commands::{
    handle_approval, handle_benchmark, handle_completion_local, handle_fork, handle_git,
    handle_logs, handle_opportunities, handle_preflight, handle_privacy_scan, handle_recipes,
    handle_restore_backup_dry_run, handle_round, handle_scorecard, handle_selftest_local,
    handle_session, handle_terminal, handle_trace, handle_usage, CommandRouter, SlashCommand,
};
use crate::config::AppConfig;
use crate::permissions::PermissionEngine;
use crate::session::{
    ApprovalStatus, PlanStepStatus, SessionStore, SideQuestion, SideQuestionStatus,
};
use crate::tools::{ToolExecutor, ToolRegistry};
use anyhow::{anyhow, Context, Result};
use std::future::Future;
use std::path::Path;

pub(super) fn handle_running_tui_local_command(state: &mut TuiState, input: &str) -> bool {
    if !state.running {
        return false;
    }
    let command = match CommandRouter::parse(input) {
        Ok(Some(command)) => command,
        Ok(None) => return false,
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "running command parse failed".to_string();
            return true;
        }
    };
    match command {
        SlashCommand::Help { args } => {
            match CommandRouter::help_for(&args) {
                Ok(output) => {
                    state.chat.push(ChatLine {
                        role: "deepcli".to_string(),
                        content: output.clone(),
                    });
                    state.last_event = format_action_event("running command ok", &output);
                }
                Err(error) => {
                    let message = error.to_string();
                    state.chat.push(ChatLine {
                        role: "error".to_string(),
                        content: message.clone(),
                    });
                    state.last_event = format_action_event("running command failed", &message);
                }
            }
            true
        }
        SlashCommand::Status { args } => {
            if args.is_empty() {
                push_running_command_result(state, format_tui_running_status);
            } else {
                let message =
                    "Agent 运行中的 `/status` 只支持无参数；请在任务空闲后使用 `/status --json` 或 `/status --output ...`。"
                        .to_string();
                state.chat.push(ChatLine {
                    role: "error".to_string(),
                    content: message.clone(),
                });
                state.last_event = format_action_event("running command failed", &message);
            }
            true
        }
        SlashCommand::Usage { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/usage")?;
                handle_usage(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Btw { args } => {
            push_running_command_result(state, |active| handle_tui_running_btw(active, args));
            true
        }
        SlashCommand::Trace { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/trace")?;
                handle_trace(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Logs { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/logs")?;
                handle_logs(&active.workspace, args)
            });
            true
        }
        SlashCommand::Privacy { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/privacy")?;
                let config = AppConfig::load_effective(&active.workspace, None)?;
                handle_privacy_scan(&active.workspace, &config, args)
            });
            true
        }
        SlashCommand::Fork { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/fork")?;
                handle_fork(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Recipes { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/recipes")?;
                let (config, registry) = running_local_product_context(&active.workspace)?;
                handle_recipes(&active.workspace, &config, &registry, args)
            });
            true
        }
        SlashCommand::Scorecard { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/scorecard")?;
                let (config, registry) = running_local_product_context(&active.workspace)?;
                handle_scorecard(&active.workspace, &config, &registry, args)
            });
            true
        }
        SlashCommand::Opportunities { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/opportunities")?;
                let (config, registry) = running_local_product_context(&active.workspace)?;
                handle_opportunities(&active.workspace, &config, &registry, args)
            });
            true
        }
        SlashCommand::Round { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/round")?;
                ensure_running_round_is_read_only(&args)?;
                let (config, registry) = running_local_product_context(&active.workspace)?;
                handle_round(&active.workspace, &config, &registry, args)
            });
            true
        }
        SlashCommand::Benchmark { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/benchmark")?;
                ensure_running_benchmark_is_read_only(&args)?;
                let (config, registry) = running_local_product_context(&active.workspace)?;
                handle_benchmark(&active.workspace, &config, &registry, args)
            });
            true
        }
        SlashCommand::Selftest { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/selftest")?;
                handle_selftest_local(&active.workspace, args)
            });
            true
        }
        SlashCommand::Preflight { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/preflight")?;
                ensure_running_preflight_is_planned(&args)?;
                handle_preflight(&active.workspace, args)
            });
            true
        }
        SlashCommand::Completion { args } => {
            push_running_command_result(state, |active| {
                ensure_running_completion_is_observation_only(&args)?;
                handle_completion_local(&active.workspace, args)
            });
            true
        }
        SlashCommand::Approval { args } => {
            push_running_command_result(state, |active| {
                ensure_running_no_output(&args, "/approval")?;
                handle_approval(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Git { args } => {
            push_running_command_result(state, |active| handle_tui_running_git(active, args));
            true
        }
        SlashCommand::Session { args } => {
            push_running_command_result(state, |active| {
                if matches!(
                    args.first().map(String::as_str),
                    Some("restore-backup" | "restore")
                ) {
                    return handle_restore_backup_dry_run(
                        &active.workspace,
                        Some(active.session_id.clone()),
                        &args[1..],
                        false,
                    );
                }
                ensure_running_session_is_read_only(&args)?;
                handle_session(&active.workspace, Some(active.session_id.clone()), args)
            });
            true
        }
        SlashCommand::Terminal { args } => {
            push_running_command_result(state, |active| handle_tui_running_terminal(active, args));
            true
        }
        SlashCommand::Stop => {
            stop_running_task(state, false, "/stop");
            true
        }
        SlashCommand::Quit => {
            stop_running_task(state, true, "/quit");
            true
        }
        _ => {
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: running_tui_supported_command_hint(),
            });
            state.last_event = "running command unsupported".to_string();
            true
        }
    }
}

pub(super) fn running_tui_supported_command_hint() -> String {
    format!(
        "Agent 正在运行；当前支持本地 {}；运行中旁路命令不写 `--output` artifact。",
        running_tui_supported_command_labels().join("、")
    )
}

pub(super) fn running_tui_deferred_input_hint() -> String {
    format!(
        "Agent 正在运行；当前可用 {} 处理旁路事项；需要写 `--output` artifact 时请等待任务结束或先 `/stop`。",
        running_tui_supported_command_labels().join("、")
    )
}

fn running_tui_supported_command_labels() -> Vec<String> {
    CommandRouter::help_summaries()
        .into_iter()
        .filter(|summary| summary.running_safe)
        .map(|summary| running_tui_command_hint_label(summary.name))
        .collect()
}

fn running_tui_command_hint_label(command: &str) -> String {
    match command {
        "/preflight" => "`/preflight --dry-run`".to_string(),
        "/git" => "read-only `/git`".to_string(),
        "/session" => {
            "read-only `/session`（含 `/session restore-backup --dry-run --json`）".to_string()
        }
        "/cleanup" => "dry-run `/cleanup`".to_string(),
        "/btw" => "`/btw ask/list/answer/clear`".to_string(),
        command => format!("`{command}`"),
    }
}

fn running_local_product_context(workspace: &Path) -> Result<(AppConfig, ToolRegistry)> {
    Ok((
        AppConfig::load_effective(workspace, None)?,
        ToolRegistry::mvp(),
    ))
}

fn running_args_include_output(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--output" | "-o") || arg.starts_with("--output="))
}

pub(super) fn ensure_running_no_output(args: &[String], command: &str) -> Result<()> {
    if running_args_include_output(args) {
        anyhow::bail!(
            "stop or wait for the running task before executing `{command} ... --output`; it writes a file"
        );
    }
    Ok(())
}

pub(super) fn ensure_running_completion_is_observation_only(args: &[String]) -> Result<()> {
    ensure_running_no_output(args, "/completion")?;
    if args.iter().any(|arg| arg == "--force") {
        anyhow::bail!(
            "completion install --force writes a shell completion file; stop or wait for the running task before installing shell completions"
        );
    }
    Ok(())
}

fn ensure_running_round_is_read_only(args: &[String]) -> Result<()> {
    if args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--run-benchmark"
                | "--run-benchmarks"
                | "--run-suite"
                | "--preset"
                | "--presets"
                | "--fail-on-command"
                | "--fail-fast"
        ) || arg.starts_with("--preset=")
            || arg.starts_with("--presets=")
    }) {
        anyhow::bail!(
            "stop or wait for the running task before executing benchmark-producing `/round` options"
        );
    }
    Ok(())
}

fn ensure_running_benchmark_is_read_only(args: &[String]) -> Result<()> {
    let Some(first) = args.first().map(String::as_str) else {
        return Ok(());
    };
    if first.starts_with('-') {
        return Ok(());
    }
    let read_only = matches!(
        first,
        "scorecard"
            | "rubric"
            | "presets"
            | "preset"
            | "catalog"
            | "status"
            | "health"
            | "doctor"
            | "gate"
            | "summary"
            | "summarize"
            | "report"
            | "trend"
            | "trends"
            | "history"
            | "compare"
            | "comparison"
            | "baseline"
            | "baselines"
            | "baseline-list"
            | "baseline-ls"
            | "list"
            | "ls"
            | "show"
            | "view"
    );
    if !read_only {
        anyhow::bail!(
            "stop or wait for the running task before executing benchmark-producing `/benchmark` options"
        );
    }
    Ok(())
}

fn ensure_running_preflight_is_planned(args: &[String]) -> Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--dry-run" | "--list" | "--plan"))
    {
        return Ok(());
    }
    anyhow::bail!(
        "stop or wait for the running task before executing `/preflight`; use `/preflight --dry-run --json` while the agent is running"
    );
}

fn ensure_running_session_is_read_only(args: &[String]) -> Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--output" | "-o") || arg.starts_with("--output="))
    {
        anyhow::bail!(
            "`/session ... --output` writes a file; wait for the running task or use `/stop` before writing session artifacts"
        );
    }
    match args.first().map(String::as_str) {
        Some("rename") => anyhow::bail!(
            "`/session rename` updates session metadata; wait for the running task or use `/stop` before renaming"
        ),
        Some("export") => anyhow::bail!(
            "`/session export` writes a file; wait for the running task or use `/stop` before exporting"
        ),
        Some("prune-empty" | "prune")
            if args.iter().any(|arg| arg == "--force") =>
        {
            anyhow::bail!(
                "`/session prune-empty --force` deletes session directories; wait for the running task or use `/stop` before cleanup"
            );
        }
        _ => Ok(()),
    }
}

fn ensure_running_git_is_read_only(args: &[String]) -> Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--output" | "-o") || arg.starts_with("--output="))
    {
        anyhow::bail!(
            "stop or wait for the running task before executing `/git ... --output`; it writes a file"
        );
    }
    let action = args
        .first()
        .filter(|value| !value.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("status");
    if matches!(action, "status" | "diff" | "branch" | "message") {
        return Ok(());
    }
    anyhow::bail!(
        "stop or wait for the running task before executing Git write action `/git {action}`"
    )
}

fn handle_tui_running_git(active: &ActiveSessionRef, args: Vec<String>) -> Result<String> {
    ensure_running_git_is_read_only(&args)?;
    let config = AppConfig::load_effective(&active.workspace, None)?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let permissions = PermissionEngine::new(
        &active.workspace,
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        &active.workspace,
        permissions,
        Some(session),
        config.agent.max_subagent_depth,
    );
    block_on_tui_local_future(handle_git(&active.workspace, &executor, args))
}

fn block_on_tui_local_future<F>(future: F) -> Result<String>
where
    F: Future<Output = Result<String>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create local runtime for running command")?;
    runtime.block_on(future)
}

fn push_running_command_result<F>(state: &mut TuiState, action: F)
where
    F: FnOnce(&ActiveSessionRef) -> Result<String>,
{
    let Some(active) = state.active_session.as_ref() else {
        let message = "当前运行会话不可用。".to_string();
        state.result_scroll = 0;
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: message.clone(),
        });
        state.last_event = format_action_event("running command failed", &message);
        return;
    };
    match action(active) {
        Ok(output) => {
            state.result_scroll = 0;
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: output.clone(),
            });
            state.last_event = format_action_event("running command ok", &output);
        }
        Err(error) => {
            let message = error.to_string();
            state.result_scroll = 0;
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: message.clone(),
            });
            state.last_event = format_action_event("running command failed", &message);
        }
    }
}

fn format_tui_running_status(active: &ActiveSessionRef) -> Result<String> {
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let activity = session.activity_summary()?;
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    let plan = session.load_plan()?.map(|plan| {
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Completed)
            .count();
        format!("{completed}/{} completed", plan.steps.len())
    });
    let latest_test = session
        .load_recent_test_runs(1)?
        .into_iter()
        .last()
        .map(|test| {
            format!(
                "latest_test={} {}",
                if test.passed { "pass" } else { "fail" },
                compact_ui_text(&test.command, 52)
            )
        })
        .unwrap_or_else(|| "latest_test=none".to_string());
    Ok(format!(
        "running session {}\nstate: {:?}\nprovider: {} model: {}\nactivity: messages={} tools={} tests={} side_questions={} approvals={} summary={}\nopen_btw={} pending_approvals={}\nplan: {}\n{}",
        session.id(),
        session.metadata.state,
        session.metadata.provider,
        session
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "<unset>".to_string()),
        activity.message_count,
        activity.tool_call_count,
        activity.test_run_count,
        activity.side_question_count,
        activity.approval_request_count,
        activity.has_summary,
        open_questions,
        pending_approvals,
        plan.unwrap_or_else(|| "none".to_string()),
        latest_test
    ))
}

fn handle_tui_running_btw(active: &ActiveSessionRef, args: Vec<String>) -> Result<String> {
    ensure_running_no_output(&args, "/btw")?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let include_all = args.iter().any(|arg| arg == "--all");
            Ok(format_tui_side_questions(
                &session.load_side_questions()?,
                include_all,
            ))
        }
        Some("ask") => {
            let question = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if question.trim().is_empty() {
                anyhow::bail!("/btw ask requires a question");
            }
            let item = session.enqueue_side_question(question.trim())?;
            Ok(format!(
                "queued by-the-way question {} while the main task keeps running: {}",
                item.id, item.question
            ))
        }
        Some("answer") => {
            let id = args
                .get(1)
                .ok_or_else(|| anyhow!("missing side question id"))?;
            let answer = args
                .iter()
                .skip(2)
                .filter(|arg| arg.as_str() != "--current")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if answer.trim().is_empty() {
                anyhow::bail!("/btw answer requires an answer");
            }
            let item = session.answer_side_question(id, answer.trim())?;
            Ok(format!("answered by-the-way question {}", item.id))
        }
        Some("clear") => {
            let cleared = session.clear_side_questions()?;
            Ok(format!("cleared {cleared} open by-the-way question(s)"))
        }
        Some(other) => anyhow::bail!("unsupported /btw action `{other}` while running"),
    }
}

fn handle_tui_running_terminal(active: &ActiveSessionRef, args: Vec<String>) -> Result<String> {
    ensure_running_no_output(&args, "/terminal")?;
    let config = AppConfig::load_effective(&active.workspace, None)?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let permissions = PermissionEngine::new(
        &active.workspace,
        config.permissions.clone(),
        config.sandbox.clone(),
    );
    let executor = ToolExecutor::new(
        &active.workspace,
        permissions,
        Some(session),
        config.agent.max_subagent_depth,
    );
    handle_terminal(&active.workspace, &executor, args)
}

fn format_tui_side_questions(items: &[SideQuestion], include_all: bool) -> String {
    let lines = items
        .iter()
        .filter(|item| include_all || item.status == SideQuestionStatus::Open)
        .map(|item| {
            let answer = item
                .answer
                .as_ref()
                .map(|answer| format!(" answer={}", compact_ui_text(answer, 60)))
                .unwrap_or_default();
            format!(
                "{} [{}] {}{}",
                short_id(&item.id.to_string()),
                tui_side_question_status_label(&item.status),
                compact_ui_text(&item.question, 86),
                answer
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "no by-the-way questions".to_string()
    } else {
        lines.join("\n")
    }
}

fn tui_side_question_status_label(status: &SideQuestionStatus) -> &'static str {
    match status {
        SideQuestionStatus::Open => "open",
        SideQuestionStatus::Answered => "answered",
        SideQuestionStatus::Cleared => "cleared",
    }
}
