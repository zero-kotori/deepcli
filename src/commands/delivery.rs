use super::*;
use crate::schema_ids;
use anyhow::{bail, Result};
use serde_json::{json, Value};

const SESSION_DIFF_FALLBACK_LIMIT: usize = 20;

struct SessionDiffSource {
    session: Session,
    note: Option<String>,
    records: Vec<SessionDiffRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffOptions {
    pub(crate) staged: bool,
    pub(crate) path_filters: Vec<String>,
    pub(crate) view: DiffView,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffView {
    Full,
    Stat,
    NameOnly,
}

pub(crate) fn parse_diff_args(args: &[String]) -> Result<DiffOptions> {
    let mut staged = false;
    let mut path_filters = Vec::new();
    let mut view = DiffView::Full;
    let mut limit = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--staged" => {
                staged = true;
                index += 1;
            }
            "--stat" | "--summary" => {
                if view == DiffView::NameOnly {
                    bail!("choose only one /diff display mode: --stat or --name-only");
                }
                view = DiffView::Stat;
                index += 1;
            }
            "--name-only" | "--names" => {
                if view == DiffView::Stat {
                    bail!("choose only one /diff display mode: --stat or --name-only");
                }
                view = DiffView::NameOnly;
                index += 1;
            }
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 20_000));
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let raw = value.trim_start_matches("--limit=");
                limit = Some(parse_positive_usize(raw, "limit")?.clamp(1, 20_000));
                index += 1;
            }
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            other => bail!("unsupported /diff option `{other}`"),
        }
    }
    Ok(DiffOptions {
        staged,
        path_filters,
        view,
        limit,
    })
}

pub(crate) fn parse_review_args(args: &[String]) -> Result<Vec<String>> {
    let mut path_filters = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            other => bail!("unsupported /review option `{other}`"),
        }
    }
    Ok(path_filters)
}

pub(crate) async fn handle_diff(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_diff_args(&args)?;
    let output = executor
        .execute("git_diff", json!({ "staged": options.staged }))
        .await?;
    let git_diff_available = command_succeeded(&output.raw);
    if git_diff_available && !output.content.trim().is_empty() {
        let diff = filter_diff_by_paths(&output.content, &options.path_filters);
        if !diff.trim().is_empty() {
            return Ok(format_diff_display(&diff, &options));
        }
        if options.staged {
            return Ok(format!(
                "no staged Git diff matched path scope {}",
                format_verify_path_filters(&options.path_filters)
            ));
        }
    }
    if options.staged {
        return Ok(output.content);
    }

    if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        current.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        return Ok(format_session_diff_display(&source, &options));
    }

    let mut report = if git_diff_available {
        "no local Git diff and no session diff records found".to_string()
    } else {
        "no Git diff available and no session diff records found".to_string()
    };
    if !git_diff_available {
        if let Some(detail) = command_failure_detail(&output) {
            report.push_str(&format!("\nnote: git diff unavailable: {detail}"));
        }
        report.push_str(
            "\nnext: run `/session diffs` after a deepcli file edit or initialize Git for workspace diff",
        );
    }
    if !options.path_filters.is_empty() {
        report.push_str(&format!(
            "\nnote: no changes matched path scope {}",
            format_verify_path_filters(&options.path_filters)
        ));
    }
    Ok(report)
}

pub(crate) async fn handle_review(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let path_filters = parse_review_args(&args)?;
    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let status = if command_succeeded(&status_output.raw) {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff_available = command_succeeded(&diff_output.raw);
    if git_diff_available && !diff_output.content.trim().is_empty() {
        let diff = filter_diff_by_paths(&diff_output.content, &path_filters);
        if !diff.trim().is_empty() {
            let mut report = scoped_report_prefix(&path_filters);
            report.push_str(&review_worktree(status, &diff));
            return Ok(report);
        }
    }

    if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        current.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &path_filters,
    )? {
        let mut report = format!("session diff review: session {}", source.session.id());
        if let Some(title) = source.session.metadata.title.as_deref() {
            report.push_str(&format!(" ({title})"));
        }
        if let Some(note) = source.note {
            report.push_str(&format!("\nnote: {note}"));
        }
        if !path_filters.is_empty() {
            report.push_str(&format!(
                "\nscope: paths={}",
                format_verify_path_filters(&path_filters)
            ));
        }
        report.push('\n');
        report.push_str(&review_worktree(
            status,
            &session_diff_review_input(&source.records),
        ));
        return Ok(report);
    }

    let mut report = review_worktree(status, "");
    if !git_diff_available {
        if let Some(detail) = command_failure_detail(&diff_output) {
            report.push_str(&format!("\nnote: git diff unavailable: {detail}"));
        }
        report.push_str(
            "\nnext: run `/session diffs` after a deepcli file edit or initialize Git for workspace diff review",
        );
    }
    if !path_filters.is_empty() {
        report.push_str(&format!(
            "\nnote: no changes matched path scope {}",
            format_verify_path_filters(&path_filters)
        ));
    }
    Ok(report)
}

pub(crate) async fn handle_verify(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_verify_args(&args, current)?;
    let store = SessionStore::new(workspace);
    let use_workspace_only_requested_test = options.session_id.is_none() && options.run_tests;
    let (session, session_note) = if use_workspace_only_requested_test {
        (None, None)
    } else {
        resolve_session_for_verify(
            &store,
            options.session_id.as_deref(),
            options.explicit_session,
        )?
    };
    let test_count_before = session
        .as_ref()
        .map(|session| session.load_test_runs().map(|tests| tests.len()))
        .transpose()?;

    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let test_run = run_verification_tests(executor, &options).await;
    let environment_checks =
        run_verification_environment_checks(executor, &options.env_checks).await;
    if let (Some(session), Some(count_before)) = (session.as_ref(), test_count_before) {
        persist_verification_test_run_if_needed(session, count_before, &test_run)?;
    }
    let status_available = command_succeeded(&status_output.raw);
    let git_diff_available = command_succeeded(&diff_output.raw);
    let status_text = if status_available {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff = if git_diff_available {
        diff_output.content.trim()
    } else {
        ""
    };

    let session_id_for_diff = session.as_ref().map(|session| session.id().to_string());
    let diff_source = if !git_diff.is_empty() {
        let scoped = filter_diff_by_paths(git_diff, &options.path_filters);
        if !scoped.trim().is_empty() {
            VerificationDiffSource::Git { diff: scoped }
        } else if let Some(source) = resolve_scoped_session_diff_source(
            workspace,
            session_id_for_diff.as_deref(),
            SESSION_DIFF_FALLBACK_LIMIT,
            &options.path_filters,
        )? {
            VerificationDiffSource::Session(source)
        } else {
            VerificationDiffSource::None {
                git_available: git_diff_available,
                detail: no_scoped_diff_detail(&options.path_filters),
            }
        }
    } else if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        session_id_for_diff.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        VerificationDiffSource::Session(source)
    } else {
        VerificationDiffSource::None {
            git_available: git_diff_available,
            detail: if git_diff_available {
                no_scoped_diff_detail(&options.path_filters)
            } else {
                command_failure_detail(&diff_output)
            },
        }
    };

    let report = format_verification_report(VerificationReportInput {
        workspace,
        session: session.as_ref(),
        session_note,
        status: VerificationStatusSource {
            available: status_available,
            text: status_text,
            detail: if status_available {
                None
            } else {
                command_failure_detail(&status_output)
            },
        },
        path_filters: &options.path_filters,
        diff_source,
        test_limit: options.limit,
        test_run,
        environment_checks: &environment_checks,
    })?;
    let output = if options.json_output {
        format_verification_report_json(&report, &environment_checks)?
    } else {
        report
    };
    if let Some(output_path) = options.output_path.as_deref() {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_blockers && verification_output_has_blockers(&output, options.json_output) {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

pub(crate) async fn handle_handoff(
    workspace: &Path,
    current: Option<String>,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_handoff_args(&args, current)?;
    let store = SessionStore::new(workspace);
    let (session, session_note) = resolve_session_for_verify(
        &store,
        options.session_id.as_deref(),
        options.explicit_session,
    )?;

    let status_output = executor.execute("git_status", json!({})).await?;
    let diff_output = executor.execute("git_diff", json!({})).await?;
    let environment_checks =
        run_verification_environment_checks(executor, &options.env_checks).await;
    let status_available = command_succeeded(&status_output.raw);
    let git_diff_available = command_succeeded(&diff_output.raw);
    let status_text = if status_available {
        status_output.content.as_str()
    } else {
        ""
    };
    let git_diff = if git_diff_available {
        diff_output.content.trim()
    } else {
        ""
    };
    let session_id_for_diff = session.as_ref().map(|session| session.id().to_string());
    let diff_source = if !git_diff.is_empty() {
        let scoped = filter_diff_by_paths(git_diff, &options.path_filters);
        if !scoped.trim().is_empty() {
            VerificationDiffSource::Git { diff: scoped }
        } else if let Some(source) = resolve_scoped_session_diff_source(
            workspace,
            session_id_for_diff.as_deref(),
            SESSION_DIFF_FALLBACK_LIMIT,
            &options.path_filters,
        )? {
            VerificationDiffSource::Session(source)
        } else {
            VerificationDiffSource::None {
                git_available: git_diff_available,
                detail: no_scoped_diff_detail(&options.path_filters),
            }
        }
    } else if let Some(source) = resolve_scoped_session_diff_source(
        workspace,
        session_id_for_diff.as_deref(),
        SESSION_DIFF_FALLBACK_LIMIT,
        &options.path_filters,
    )? {
        VerificationDiffSource::Session(source)
    } else {
        VerificationDiffSource::None {
            git_available: git_diff_available,
            detail: if git_diff_available {
                no_scoped_diff_detail(&options.path_filters)
            } else {
                command_failure_detail(&diff_output)
            },
        }
    };

    let report = format_handoff_report(HandoffReportInput {
        workspace,
        session: session.as_ref(),
        session_note,
        status: VerificationStatusSource {
            available: status_available,
            text: status_text,
            detail: if status_available {
                None
            } else {
                command_failure_detail(&status_output)
            },
        },
        path_filters: &options.path_filters,
        diff_source,
        limit: options.limit,
        environment_checks: &environment_checks,
    })?;
    let has_blockers = !handoff_report_blockers(&report).is_empty();
    let output = match options.format {
        HandoffFormat::Text => Ok(report),
        HandoffFormat::Markdown => Ok(format_handoff_report_markdown(&report)),
        HandoffFormat::PullRequest => Ok(format_handoff_report_pr_description(&report)),
        HandoffFormat::Json => format_handoff_report_json(&report, &environment_checks),
    }?;
    if let Some(output_path) = options.output_path.as_deref() {
        write_command_output(workspace, output_path, &output)?;
    }
    if options.fail_on_blockers && has_blockers {
        return Err(CommandExit::new(output, 1).into());
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifyOptions {
    pub(crate) limit: usize,
    pub(crate) session_id: Option<String>,
    pub(crate) explicit_session: bool,
    pub(crate) run_tests: bool,
    pub(crate) test_command: Option<String>,
    pub(crate) env_checks: Vec<String>,
    pub(crate) path_filters: Vec<String>,
    pub(crate) fail_on_blockers: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
}

pub(crate) struct HandoffOptions {
    pub(crate) limit: usize,
    pub(crate) session_id: Option<String>,
    pub(crate) explicit_session: bool,
    pub(crate) path_filters: Vec<String>,
    pub(crate) env_checks: Vec<String>,
    pub(crate) format: HandoffFormat,
    pub(crate) fail_on_blockers: bool,
    pub(crate) output_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandoffFormat {
    Text,
    Markdown,
    PullRequest,
    Json,
}

pub(crate) fn parse_verify_args(args: &[String], current: Option<String>) -> Result<VerifyOptions> {
    let mut limit = 5usize;
    let mut session_id = None;
    let mut explicit_session = false;
    let mut run_tests = false;
    let mut test_command = None;
    let mut env_checks = Vec::new();
    let mut path_filters = Vec::new();
    let mut fail_on_blockers = false;
    let mut json_output = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 100);
                index += 2;
            }
            "--run-tests" => {
                run_tests = true;
                index += 1;
            }
            "--fail-on-blockers" | "--strict" => {
                fail_on_blockers = true;
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
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                let path = value.trim_start_matches("--path=");
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                let path = value.trim_start_matches("--scope=");
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 1;
            }
            "--test-command" => {
                let command = required_arg(args, index + 1, "test command")?;
                test_command = Some(command.to_string());
                run_tests = true;
                index += 2;
            }
            value if value.starts_with("--test-command=") => {
                let command = value
                    .trim_start_matches("--test-command=")
                    .trim()
                    .to_string();
                if command.is_empty() {
                    bail!("--test-command requires a command");
                }
                test_command = Some(command);
                run_tests = true;
                index += 1;
            }
            "--env-check" | "--env" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with('-'))
                {
                    let target = args[index + 1].trim();
                    validate_env_target(target, false)?;
                    env_checks.push(target.to_string());
                    index += 2;
                } else {
                    env_checks.push("docker".to_string());
                    index += 1;
                }
            }
            value if value.starts_with("--env-check=") => {
                let target = value.trim_start_matches("--env-check=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            value if value.starts_with("--env=") => {
                let target = value.trim_start_matches("--env=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            "--" => {
                let command = args[index + 1..].join(" ").trim().to_string();
                if command.is_empty() {
                    bail!("-- requires a test command after it");
                }
                test_command = Some(command);
                run_tests = true;
                break;
            }
            "--current" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit_session = true;
                index += 1;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                limit = parse_positive_usize(value, "limit")?.clamp(1, 100);
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /verify option `{value}`"),
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }

    Ok(VerifyOptions {
        limit,
        session_id: session_id.or(current),
        explicit_session,
        run_tests,
        test_command,
        env_checks: dedup_preserve_order(env_checks),
        path_filters,
        fail_on_blockers,
        json_output,
        output_path,
    })
}

pub(crate) fn parse_handoff_args(
    args: &[String],
    current: Option<String>,
) -> Result<HandoffOptions> {
    let mut limit = 8usize;
    let mut session_id = None;
    let mut explicit_session = false;
    let mut path_filters = Vec::new();
    let mut env_checks = Vec::new();
    let mut format = HandoffFormat::Text;
    let mut explicit_format = false;
    let mut fail_on_blockers = false;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" | "-n" => {
                let raw = required_arg(args, index + 1, "limit")?;
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 100);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let raw = value.trim_start_matches("--limit=");
                limit = parse_positive_usize(raw, "limit")?.clamp(1, 100);
                index += 1;
            }
            "--path" | "--scope" => {
                let path = required_arg(args, index + 1, "path")?;
                path_filters.push(normalize_scope_path_filter(path)?);
                index += 2;
            }
            value if value.starts_with("--path=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--path="),
                )?);
                index += 1;
            }
            value if value.starts_with("--scope=") => {
                path_filters.push(normalize_scope_path_filter(
                    value.trim_start_matches("--scope="),
                )?);
                index += 1;
            }
            "--env-check" | "--env" => {
                if args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with('-'))
                {
                    let target = args[index + 1].trim();
                    validate_env_target(target, false)?;
                    env_checks.push(target.to_string());
                    index += 2;
                } else {
                    env_checks.push("docker".to_string());
                    index += 1;
                }
            }
            value if value.starts_with("--env-check=") => {
                let target = value.trim_start_matches("--env-check=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            value if value.starts_with("--env=") => {
                let target = value.trim_start_matches("--env=").trim();
                validate_env_target(target, false)?;
                env_checks.push(target.to_string());
                index += 1;
            }
            "--markdown" | "--md" => {
                set_handoff_format(&mut format, &mut explicit_format, HandoffFormat::Markdown)?;
                index += 1;
            }
            "--pr" | "--pr-description" => {
                set_handoff_format(
                    &mut format,
                    &mut explicit_format,
                    HandoffFormat::PullRequest,
                )?;
                index += 1;
            }
            "--json" => {
                set_handoff_format(&mut format, &mut explicit_format, HandoffFormat::Json)?;
                index += 1;
            }
            "--fail-on-blockers" | "--strict" => {
                fail_on_blockers = true;
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
            "--format" => {
                let raw = required_arg(args, index + 1, "format")?;
                set_handoff_format(
                    &mut format,
                    &mut explicit_format,
                    parse_handoff_format(raw)?,
                )?;
                index += 2;
            }
            value if value.starts_with("--format=") => {
                set_handoff_format(
                    &mut format,
                    &mut explicit_format,
                    parse_handoff_format(value.trim_start_matches("--format="))?,
                )?;
                index += 1;
            }
            "--current" => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(
                    current
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                explicit_session = true;
                index += 1;
            }
            value if index == 0 && value.parse::<usize>().is_ok() => {
                limit = parse_positive_usize(value, "limit")?.clamp(1, 100);
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /handoff option `{value}`"),
            value => {
                if session_id.is_some() {
                    bail!("multiple session ids were provided");
                }
                session_id = Some(value.to_string());
                explicit_session = true;
                index += 1;
            }
        }
    }

    Ok(HandoffOptions {
        limit,
        session_id: session_id.or(current),
        explicit_session,
        path_filters,
        env_checks: dedup_preserve_order(env_checks),
        format,
        fail_on_blockers,
        output_path,
    })
}

fn parse_handoff_format(raw: &str) -> Result<HandoffFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" | "plain" => Ok(HandoffFormat::Text),
        "markdown" | "md" => Ok(HandoffFormat::Markdown),
        "pr" | "pull-request" | "pull_request" | "pr-description" | "pr_description" => {
            Ok(HandoffFormat::PullRequest)
        }
        "json" => Ok(HandoffFormat::Json),
        value => bail!("unsupported /handoff format `{value}`"),
    }
}

fn set_handoff_format(
    current: &mut HandoffFormat,
    explicit: &mut bool,
    next: HandoffFormat,
) -> Result<()> {
    if *explicit && *current != next {
        bail!("conflicting /handoff output format options");
    }
    *current = next;
    *explicit = true;
    Ok(())
}

fn normalize_scope_path_filter(raw: &str) -> Result<String> {
    let mut path = raw.trim().replace('\\', "/");
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped.to_string();
    }
    while path.ends_with('/') {
        path.pop();
    }
    if path.is_empty() || path == "." {
        bail!("--path requires a workspace-relative path");
    }
    if path.starts_with('/') {
        bail!("--path must be workspace-relative: {raw}");
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("--path must not contain empty, `.` or `..` segments: {raw}");
    }
    Ok(path)
}

pub(crate) fn filter_diff_by_paths(diff: &str, filters: &[String]) -> String {
    if filters.is_empty() || diff.trim().is_empty() {
        return diff.to_string();
    }

    let mut output = Vec::new();
    let mut section = Vec::new();
    let mut include_section = false;
    let mut in_section = false;

    for line in diff.lines() {
        if let Some(path) = diff_section_path_from_line(line) {
            if in_section && include_section {
                output.append(&mut section);
            } else {
                section.clear();
            }
            include_section = path_matches_verify_filters(&path, filters);
            in_section = true;
            section.push(line.to_string());
        } else if in_section {
            section.push(line.to_string());
        }
    }

    if in_section && include_section {
        output.extend(section);
    }
    output.join("\n")
}

fn diff_section_path_from_line(line: &str) -> Option<String> {
    if line.starts_with("diff --git ") || line.starts_with("diff --session ") {
        review_path_from_diff_line(line)
    } else {
        None
    }
}

fn path_matches_verify_filters(path: &str, filters: &[String]) -> bool {
    let Some(path) = normalize_diff_path_for_filter(path) else {
        return false;
    };
    filters
        .iter()
        .any(|filter| path == *filter || path.starts_with(&format!("{filter}/")))
}

fn normalize_diff_path_for_filter(raw: &str) -> Option<String> {
    let mut path = raw.trim().trim_matches('"').replace('\\', "/");
    if path == "/dev/null" || path.is_empty() {
        return None;
    }
    if let Some(stripped) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        path = stripped.to_string();
    }
    while let Some(stripped) = path.strip_prefix("./") {
        path = stripped.to_string();
    }
    Some(path)
}

fn format_verify_path_filters(filters: &[String]) -> String {
    filters.join(", ")
}

fn format_path_scope_args(filters: &[String]) -> String {
    filters
        .iter()
        .map(|filter| format!(" --path {}", shell_words::quote(filter)))
        .collect::<String>()
}

fn scoped_report_prefix(filters: &[String]) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        format!("scope: paths={}\n", format_verify_path_filters(filters))
    }
}

fn format_diff_display(diff: &str, options: &DiffOptions) -> String {
    match options.view {
        DiffView::Full => limit_display_lines(diff, options.limit, "diff"),
        DiffView::Stat => format_diff_stat(diff, options.limit),
        DiffView::NameOnly => format_diff_name_only(diff, options.limit),
    }
}

fn format_session_diff_display(source: &SessionDiffSource, options: &DiffOptions) -> String {
    if options.view == DiffView::Full {
        let mut output = format_session_diff_fallback(source);
        if !options.path_filters.is_empty() {
            output.push_str(&format!(
                "\nscope: paths={}",
                format_verify_path_filters(&options.path_filters)
            ));
        }
        return limit_display_lines(&output, options.limit, "session diff");
    }

    let mut output = session_diff_fallback_header(source);
    if !options.path_filters.is_empty() {
        output.push_str(&format!(
            "\nscope: paths={}",
            format_verify_path_filters(&options.path_filters)
        ));
    }
    output.push('\n');

    let body = match options.view {
        DiffView::Full => unreachable!("full session diff view returns above"),
        DiffView::Stat => {
            format_diff_stat(&session_diff_review_input(&source.records), options.limit)
        }
        DiffView::NameOnly => {
            format_diff_name_only(&session_diff_review_input(&source.records), options.limit)
        }
    };
    output.push_str(&body);
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffFileSummary {
    path: String,
    added: usize,
    removed: usize,
}

fn diff_file_summaries(diff: &str) -> Vec<DiffFileSummary> {
    let mut summaries = Vec::new();
    let mut current: Option<DiffFileSummary> = None;

    for line in diff.lines() {
        if let Some(path) = diff_section_path_from_line(line) {
            if let Some(summary) = current.take() {
                summaries.push(summary);
            }
            current = Some(DiffFileSummary {
                path,
                added: 0,
                removed: 0,
            });
            continue;
        }

        if let Some(summary) = current.as_mut() {
            if is_added_diff_line(line) {
                summary.added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                summary.removed += 1;
            }
        }
    }

    if let Some(summary) = current {
        summaries.push(summary);
    }
    summaries
}

pub(crate) fn format_diff_stat(diff: &str, limit: Option<usize>) -> String {
    let summaries = diff_file_summaries(diff);
    if summaries.is_empty() {
        return "diff stat: no file sections found".to_string();
    }

    let total_added = summaries.iter().map(|summary| summary.added).sum::<usize>();
    let total_removed = summaries
        .iter()
        .map(|summary| summary.removed)
        .sum::<usize>();
    let mut lines = vec![format!(
        "diff stat: {} file(s), +{} -{}",
        summaries.len(),
        total_added,
        total_removed
    )];
    append_limited_diff_entries(&mut lines, &summaries, limit, |summary| {
        format!("- {} +{} -{}", summary.path, summary.added, summary.removed)
    });
    lines.join("\n")
}

pub(crate) fn format_diff_name_only(diff: &str, limit: Option<usize>) -> String {
    let mut summaries = diff_file_summaries(diff);
    summaries.dedup_by(|left, right| left.path == right.path);
    if summaries.is_empty() {
        return "diff files: no file sections found".to_string();
    }

    let mut lines = vec![format!("diff files: {} file(s)", summaries.len())];
    append_limited_diff_entries(&mut lines, &summaries, limit, |summary| {
        format!("- {}", summary.path)
    });
    lines.join("\n")
}

fn append_limited_diff_entries<F>(
    lines: &mut Vec<String>,
    summaries: &[DiffFileSummary],
    limit: Option<usize>,
    mut format_entry: F,
) where
    F: FnMut(&DiffFileSummary) -> String,
{
    let shown = limit.unwrap_or(summaries.len()).min(summaries.len());
    lines.extend(summaries.iter().take(shown).map(&mut format_entry));
    if summaries.len() > shown {
        lines.push(format!(
            "... {} more file(s). Increase with `/diff --limit {}`.",
            summaries.len() - shown,
            summaries.len()
        ));
    }
}

fn limit_display_lines(text: &str, limit: Option<usize>, label: &str) -> String {
    let Some(limit) = limit else {
        return text.to_string();
    };
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= limit {
        return text.to_string();
    }

    let mut output = lines.into_iter().take(limit).collect::<Vec<_>>().join("\n");
    output.push_str(&format!(
        "\n[deepcli {label} truncated: kept {limit} of {} line(s). Increase with `/diff --limit {}` or inspect `/diff --stat` first.]",
        text.lines().count(),
        text.lines().count()
    ));
    output
}

fn no_scoped_diff_detail(filters: &[String]) -> Option<String> {
    (!filters.is_empty()).then(|| {
        format!(
            "no changes matched path scope {}",
            format_verify_path_filters(filters)
        )
    })
}

fn resolve_session_for_verify(
    store: &SessionStore,
    id: Option<&str>,
    explicit: bool,
) -> Result<(Option<Session>, Option<String>)> {
    if let Some(id) = id {
        let session = store.load(id)?;
        if explicit || session_has_recorded_activity(&session)? {
            return Ok((Some(session), None));
        }
        if let Some((candidate, _, _)) = latest_session_with_recorded_activity(store, Some(id))? {
            return Ok((
                Some(candidate),
                Some(format!(
                    "latest session with recorded activity; current session {id} had none"
                )),
            ));
        }
        return Ok((Some(session), None));
    }

    if let Some((session, _, _)) = latest_session_with_recorded_activity(store, None)? {
        return Ok((
            Some(session),
            Some("latest session with recorded activity; no current session".to_string()),
        ));
    }
    Ok((None, None))
}

pub(crate) enum VerificationDiffSource {
    Git {
        diff: String,
    },
    Session(SessionDiffSource),
    None {
        git_available: bool,
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationTestRun {
    NotRequested,
    Completed {
        command: String,
        passed: bool,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationEnvironmentCheck {
    Completed {
        target: String,
        report: EnvironmentReport,
        text: String,
    },
    Error {
        target: String,
        error: String,
    },
}

async fn run_verification_tests(
    executor: &ToolExecutor,
    options: &VerifyOptions,
) -> VerificationTestRun {
    if !options.run_tests {
        return VerificationTestRun::NotRequested;
    }
    let args = options
        .test_command
        .as_ref()
        .map(|command| json!({ "command": command }))
        .unwrap_or_else(|| json!({}));
    match executor.execute("run_tests", args).await {
        Ok(output) => verification_test_run_from_output(&output.raw),
        Err(error) => VerificationTestRun::Error(error.to_string()),
    }
}

async fn run_verification_environment_checks(
    executor: &ToolExecutor,
    targets: &[String],
) -> Vec<VerificationEnvironmentCheck> {
    let mut checks = Vec::new();
    for target in targets {
        match executor
            .execute("check_environment", json!({ "target": target }))
            .await
        {
            Ok(output) => match serde_json::from_value::<EnvironmentReport>(output.raw.clone()) {
                Ok(report) => checks.push(VerificationEnvironmentCheck::Completed {
                    target: target.clone(),
                    report,
                    text: output.content,
                }),
                Err(error) => checks.push(VerificationEnvironmentCheck::Error {
                    target: target.clone(),
                    error: format!("failed to parse environment report: {error}"),
                }),
            },
            Err(error) => checks.push(VerificationEnvironmentCheck::Error {
                target: target.clone(),
                error: error.to_string(),
            }),
        }
    }
    checks
}

fn verification_test_run_from_output(raw: &Value) -> VerificationTestRun {
    let passed = raw.get("passed").and_then(Value::as_bool).unwrap_or(false);
    let output = raw.get("output").unwrap_or(&Value::Null);
    VerificationTestRun::Completed {
        command: output
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
            .to_string(),
        passed,
        exit_code: output
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        stdout: output
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: output
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn persist_verification_test_run_if_needed(
    session: &Session,
    count_before: usize,
    test_run: &VerificationTestRun,
) -> Result<()> {
    let VerificationTestRun::Completed {
        command,
        passed,
        exit_code,
        stdout,
        stderr,
    } = test_run
    else {
        return Ok(());
    };
    if session.load_test_runs()?.len() != count_before {
        return Ok(());
    }
    session.append_test_run(&TestRunRecord {
        command: command.clone(),
        exit_code: *exit_code,
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        passed: *passed,
        created_at: Utc::now(),
    })
}

pub(crate) fn weak_test_command_reason(command: &str) -> Option<&'static str> {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Some("empty command does not exercise project behavior");
    }
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if matches!(compact.as_str(), ":" | "true" | "exit 0") {
        return Some("no-op command does not exercise project behavior");
    }
    if compact.starts_with("printf ") || compact.starts_with("echo ") {
        return Some("prints fixed output instead of exercising project behavior");
    }
    if compact.contains(" printf ok")
        || compact.contains(" echo ok")
        || compact.contains(" printf 'ok'")
        || compact.contains(" echo 'ok'")
        || compact.contains(" printf \"ok\"")
        || compact.contains(" echo \"ok\"")
    {
        return Some("prints fixed output instead of exercising project behavior");
    }
    None
}

fn weak_test_command_blocker(command: &str, context: &str) -> Option<String> {
    weak_test_command_reason(command).map(|reason| {
        format!(
            "{context} is weak test evidence: command=`{}` ({reason})",
            truncate_display(command, 120)
        )
    })
}

fn verification_test_run_weak_blocker(test_run: &VerificationTestRun) -> Option<String> {
    match test_run {
        VerificationTestRun::Completed {
            command,
            passed: true,
            ..
        } => weak_test_command_blocker(command, "requested verification test"),
        _ => None,
    }
}

fn has_strong_passing_test(tests: &[TestRunRecord]) -> bool {
    tests
        .iter()
        .any(|test| test.passed && weak_test_command_reason(&test.command).is_none())
}

fn has_strong_passing_verification_test(test_run: &VerificationTestRun) -> bool {
    matches!(
        test_run,
        VerificationTestRun::Completed {
            command,
            passed: true,
            ..
        } if weak_test_command_reason(command).is_none()
    )
}

fn latest_strong_passing_test_at(tests: &[TestRunRecord]) -> Option<DateTime<Utc>> {
    tests
        .iter()
        .filter(|test| test.passed && weak_test_command_reason(&test.command).is_none())
        .map(|test| test.created_at)
        .max()
}

fn latest_session_diff_modified_at(records: &[SessionDiffRecord]) -> Option<DateTime<Utc>> {
    records.iter().map(|record| record.modified_at).max()
}

fn latest_workspace_diff_mtime(workspace: &Path, diff: &str) -> Option<DateTime<Utc>> {
    let mut latest: Option<DateTime<Utc>> = None;
    for summary in diff_file_summaries(diff) {
        let Some(path) = normalize_diff_path_for_filter(&summary.path) else {
            continue;
        };
        let target = workspace.join(path);
        let Ok(metadata) = fs::metadata(target) else {
            continue;
        };
        let modified_at =
            DateTime::<Utc>::from(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        latest = Some(latest.map_or(modified_at, |current| current.max(modified_at)));
    }
    latest
}

fn stale_strong_test_evidence_blocker(
    tests: &[TestRunRecord],
    latest_change_at: Option<DateTime<Utc>>,
    path_filters: &[String],
) -> Option<String> {
    let latest_test_at = latest_strong_passing_test_at(tests)?;
    let latest_change_at = latest_change_at?;
    if latest_test_at >= latest_change_at {
        return None;
    }
    let scope = if path_filters.is_empty() {
        "diff".to_string()
    } else {
        "scoped diff".to_string()
    };
    Some(format!(
        "no strong passing test evidence after latest {scope} change"
    ))
}

pub(crate) struct VerificationStatusSource<'a> {
    pub(crate) available: bool,
    pub(crate) text: &'a str,
    pub(crate) detail: Option<String>,
}

pub(crate) struct VerificationReportInput<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) session: Option<&'a Session>,
    pub(crate) session_note: Option<String>,
    pub(crate) status: VerificationStatusSource<'a>,
    pub(crate) path_filters: &'a [String],
    pub(crate) diff_source: VerificationDiffSource,
    pub(crate) test_limit: usize,
    pub(crate) test_run: VerificationTestRun,
    pub(crate) environment_checks: &'a [VerificationEnvironmentCheck],
}

pub(crate) struct HandoffReportInput<'a> {
    pub(crate) workspace: &'a Path,
    pub(crate) session: Option<&'a Session>,
    pub(crate) session_note: Option<String>,
    pub(crate) status: VerificationStatusSource<'a>,
    pub(crate) path_filters: &'a [String],
    pub(crate) diff_source: VerificationDiffSource,
    pub(crate) limit: usize,
    pub(crate) environment_checks: &'a [VerificationEnvironmentCheck],
}

pub(crate) fn format_verification_report(input: VerificationReportInput<'_>) -> Result<String> {
    let mut lines = vec![
        "verification report".to_string(),
        format!("workspace: {}", input.workspace.display()),
    ];

    if let Some(session) = input.session {
        let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
        let model = session.metadata.model.as_deref().unwrap_or("<default>");
        let mut session_line = format!(
            "session: id={} full={} title={} state={:?} provider={} model={}",
            short_id(&session.id()),
            session.id(),
            title,
            session.metadata.state,
            session.metadata.provider,
            model
        );
        if let Some(note) = input.session_note {
            session_line.push_str(&format!(" ({note})"));
        }
        lines.push(session_line);
    } else {
        lines.push("session: none found; report uses workspace state only".to_string());
    }

    lines.push(format_git_status_summary(
        input.status.available,
        input.status.text,
        input.status.detail,
    ));
    if !input.path_filters.is_empty() {
        lines.push(format!(
            "scope: paths={}",
            format_verify_path_filters(input.path_filters)
        ));
    }

    let mut blockers = Vec::new();
    let workspace_only_strong_test =
        input.session.is_none() && has_strong_passing_verification_test(&input.test_run);
    if let Some(session) = input.session {
        blockers.extend(session_verification_blockers(session)?);
    } else if workspace_only_strong_test {
        lines.push(
            "session evidence: none found; using workspace-only evidence from this report"
                .to_string(),
        );
    } else {
        blockers.push("no session context found; run deepcli on the task before relying on session-level evidence".to_string());
    }

    let (diff_label, review_input, latest_change_at) = match input.diff_source {
        VerificationDiffSource::Git { diff } => {
            let (added, removed) = diff_line_counts(&diff);
            let latest_change_at = latest_workspace_diff_mtime(input.workspace, &diff);
            let label = if input.path_filters.is_empty() {
                format!("git diff with +{added} -{removed} changed line(s)")
            } else {
                format!(
                    "git diff scoped to {} with +{} -{} changed line(s)",
                    format_verify_path_filters(input.path_filters),
                    added,
                    removed
                )
            };
            (label, Some(diff), latest_change_at)
        }
        VerificationDiffSource::Session(source) => {
            let latest_change_at = latest_session_diff_modified_at(&source.records);
            let review_input = session_diff_review_input(&source.records);
            let (added, removed) = diff_line_counts(&review_input);
            let scope = if input.path_filters.is_empty() {
                String::new()
            } else {
                format!(
                    " scoped to {}",
                    format_verify_path_filters(input.path_filters)
                )
            };
            let mut label = format!(
                "session diff fallback from {}{} with {} record(s), +{} -{} changed line(s)",
                source.session.id(),
                scope,
                source.records.len(),
                added,
                removed
            );
            if let Some(note) = source.note {
                label.push_str(&format!(" ({note})"));
            }
            (label, Some(review_input), latest_change_at)
        }
        VerificationDiffSource::None {
            git_available,
            detail,
        } => {
            let mut label = if git_available {
                "none: no local Git diff and no session diff records found".to_string()
            } else {
                "none: Git diff unavailable and no session diff records found".to_string()
            };
            if let Some(detail) = detail {
                label.push_str(&format!(" ({detail})"));
            }
            (label, None, None)
        }
    };
    lines.push(format!("diff source: {diff_label}"));

    lines.push("review:".to_string());
    let review = review_worktree(
        input.status.text,
        review_input.as_deref().unwrap_or_default(),
    );
    let review_risk = review_risk_summary_from_report(&review);
    if review_risk.high_findings > 0 {
        blockers.push(format!(
            "auto-reviewer reported {} high-risk finding type(s)",
            review_risk.high_findings
        ));
    }
    if review_risk.medium_findings > 0 {
        lines.push(format!(
            "review warnings: auto-reviewer reported {} medium-risk finding type(s); inspect before handoff",
            review_risk.medium_findings
        ));
    }
    lines.push(indent_text(&truncate_display(&review, 1_500), "  "));

    lines.push("tests:".to_string());
    let test_run_succeeded = matches!(
        &input.test_run,
        VerificationTestRun::Completed { passed: true, .. }
    );
    let test_run_failed = matches!(
        &input.test_run,
        VerificationTestRun::Completed { passed: false, .. } | VerificationTestRun::Error(_)
    );
    append_verification_test_run(&mut lines, &input.test_run);
    if let Some(blocker) = verification_test_run_weak_blocker(&input.test_run) {
        lines.push(format!("  evidence warning: {blocker}"));
        blockers.push(blocker);
    }
    if let Some(session) = input.session {
        let tests = session.load_recent_test_runs(input.test_limit)?;
        let failed = tests.iter().filter(|item| !item.passed).count();
        let passed = tests.iter().filter(|item| item.passed).count();
        lines.push(format!(
            "  latest {} recorded test run(s): passed={} failed={}",
            tests.len(),
            passed,
            failed
        ));
        if let Some(latest) = tests.last() {
            lines.push(format!(
                "  latest: [{}] exit={:?} command={}",
                if latest.passed { "passed" } else { "failed" },
                latest.exit_code,
                latest.command
            ));
            if let Some(reason) = weak_test_command_reason(&latest.command) {
                lines.push(format!(
                    "  evidence warning: latest recorded test command is weak evidence ({reason})"
                ));
            }
        }
        if tests.is_empty() && !test_run_succeeded {
            blockers.push("no test runs recorded for the selected session".to_string());
        }
        if !tests.is_empty()
            && !has_strong_passing_test(&tests)
            && !has_strong_passing_verification_test(&input.test_run)
        {
            blockers.push(
                "no strong passing test evidence recorded for the selected session".to_string(),
            );
        }
        if has_strong_passing_test(&tests) && !has_strong_passing_verification_test(&input.test_run)
        {
            if let Some(blocker) =
                stale_strong_test_evidence_blocker(&tests, latest_change_at, input.path_filters)
            {
                lines.push(format!("  evidence warning: {blocker}"));
                blockers.push(blocker);
            }
        }
        if test_run_failed {
            blockers.push("requested verification test run failed".to_string());
        }
    } else {
        lines.push("  no session selected; no recorded tests available".to_string());
        if test_run_failed {
            blockers.push("requested verification test run failed".to_string());
        }
    }

    lines.push("environment:".to_string());
    append_verification_environment(
        &mut lines,
        &mut blockers,
        input.environment_checks,
        "  not requested; use `/verify --env-check docker` or `/verify --env-check compiler` when environment readiness matters",
    );

    if blockers.is_empty() {
        lines.push("blockers: none detected from recorded session signals".to_string());
    } else {
        lines.push("blockers:".to_string());
        lines.extend(blockers.iter().map(|item| format!("- {item}")));
    }

    lines.push("next actions:".to_string());
    let needs_fresh_test_evidence = blockers
        .iter()
        .any(|item| blocker_needs_fresh_test_evidence(item));
    lines.extend(verification_next_actions(
        input.session,
        review_input.is_some(),
        blockers.is_empty(),
        !matches!(input.test_run, VerificationTestRun::NotRequested),
        needs_fresh_test_evidence,
        input.path_filters,
        input.environment_checks,
    ));
    Ok(lines.join("\n"))
}

pub(crate) fn format_handoff_report(input: HandoffReportInput<'_>) -> Result<String> {
    let mut lines = vec![
        "handoff report".to_string(),
        "summary:".to_string(),
        format!("- workspace: {}", input.workspace.display()),
    ];

    if let Some(session) = input.session {
        let title = session.metadata.title.as_deref().unwrap_or("<untitled>");
        let model = session.metadata.model.as_deref().unwrap_or("<default>");
        let mut line = format!(
            "- session: id={} full={} title={} state={:?} provider={} model={}",
            short_id(&session.id()),
            session.id(),
            title,
            session.metadata.state,
            session.metadata.provider,
            model
        );
        if let Some(note) = input.session_note {
            line.push_str(&format!(" ({note})"));
        }
        lines.push(line);
    } else {
        lines.push("- session: none found; report uses workspace state only".to_string());
    }
    lines.push(format!(
        "- git: {}",
        format_git_status_summary(
            input.status.available,
            input.status.text,
            input.status.detail
        )
        .trim_start_matches("git status: ")
    ));
    if !input.path_filters.is_empty() {
        lines.push(format!(
            "- scope: paths={}",
            format_verify_path_filters(input.path_filters)
        ));
    }

    let (diff_label, review_input, latest_change_at) =
        handoff_diff_label_and_review_input(input.workspace, input.diff_source, input.path_filters);
    lines.push(format!("- diff: {diff_label}"));

    lines.push("changed files:".to_string());
    if let Some(diff) = review_input.as_deref() {
        lines.push(indent_text(
            &format_diff_stat(diff, Some(input.limit)),
            "  ",
        ));
    } else {
        lines.push("  none".to_string());
    }

    let review = review_worktree(
        input.status.text,
        review_input.as_deref().unwrap_or_default(),
    );
    let review_risk = review_risk_summary_from_report(&review);
    lines.push("review:".to_string());
    lines.push(format!(
        "  risk: high={} medium={}",
        review_risk.high_findings, review_risk.medium_findings
    ));
    lines.push(indent_text(&truncate_display(&review, 1_000), "  "));

    let mut blockers = Vec::new();
    lines.push("tests:".to_string());
    if let Some(session) = input.session {
        blockers.extend(session_verification_blockers(session)?);
        let tests = session.load_recent_test_runs(input.limit)?;
        let passed = tests.iter().filter(|test| test.passed).count();
        let failed = tests.iter().filter(|test| !test.passed).count();
        lines.push(format!(
            "  latest {} recorded test run(s): passed={} failed={}",
            tests.len(),
            passed,
            failed
        ));
        if let Some(latest) = tests.last() {
            lines.push(format!(
                "  latest: [{}] exit={:?} command={}",
                if latest.passed { "passed" } else { "failed" },
                latest.exit_code,
                latest.command
            ));
            if let Some(reason) = weak_test_command_reason(&latest.command) {
                lines.push(format!(
                    "  evidence warning: latest recorded test command is weak evidence ({reason})"
                ));
            }
        }
        if tests.is_empty() {
            blockers.push("no test runs recorded for the selected session".to_string());
        } else if !has_strong_passing_test(&tests) {
            blockers.push(
                "no strong passing test evidence recorded for the selected session".to_string(),
            );
        } else if let Some(blocker) =
            stale_strong_test_evidence_blocker(&tests, latest_change_at, input.path_filters)
        {
            lines.push(format!("  evidence warning: {blocker}"));
            blockers.push(blocker);
        }
    } else {
        lines.push("  no session selected; no recorded tests available".to_string());
        blockers
            .push("no session context found; run deepcli on the task before handoff".to_string());
    }

    lines.push("environment:".to_string());
    append_verification_environment(
        &mut lines,
        &mut blockers,
        input.environment_checks,
        "  not requested; use `/handoff --env-check docker` or `/handoff --env-check compiler` when environment readiness matters",
    );

    if review_risk.high_findings > 0 {
        blockers.push(format!(
            "auto-reviewer reported {} high-risk finding type(s)",
            review_risk.high_findings
        ));
    }
    if review_input.is_none() {
        blockers.push("no diff evidence found".to_string());
    }

    lines.push("risks and blockers:".to_string());
    if blockers.is_empty() {
        lines.push("  none detected from recorded session signals".to_string());
    } else {
        lines.extend(blockers.iter().map(|item| format!("  - {item}")));
    }

    let scope_args = format_path_scope_args(input.path_filters);
    lines.push("next actions:".to_string());
    if review_risk.medium_findings > 0 {
        lines.push(format!(
            "  - inspect review warnings: `/review{scope_args}`"
        ));
    }
    if review_input.is_some() {
        lines.push(format!(
            "  - inspect changed files: `/diff --stat{scope_args}`"
        ));
    }
    if blockers
        .iter()
        .any(|item| blocker_needs_fresh_test_evidence(item))
    {
        let scope_args = format_path_scope_args(input.path_filters);
        lines.push(
            format!(
                "  - add strong test evidence: `/verify --test-command 'cargo test'{scope_args}` or `/verify --run-tests{scope_args}`"
            ),
        );
    }
    append_handoff_environment_next_actions(&mut lines, input.environment_checks);
    if blockers.is_empty() {
        lines.push("  - generate commit message: `/git message`".to_string());
        lines.push("  - commit when ready: `/git commit <message>`".to_string());
    } else {
        lines.push("  - resolve blockers, then rerun `/handoff`".to_string());
    }

    Ok(lines.join("\n"))
}

fn handoff_diff_label_and_review_input(
    workspace: &Path,
    diff_source: VerificationDiffSource,
    path_filters: &[String],
) -> (String, Option<String>, Option<DateTime<Utc>>) {
    match diff_source {
        VerificationDiffSource::Git { diff } => {
            let (added, removed) = diff_line_counts(&diff);
            let latest_change_at = latest_workspace_diff_mtime(workspace, &diff);
            let label = if path_filters.is_empty() {
                format!("git diff with +{added} -{removed} changed line(s)")
            } else {
                format!(
                    "git diff scoped to {} with +{} -{} changed line(s)",
                    format_verify_path_filters(path_filters),
                    added,
                    removed
                )
            };
            (label, Some(diff), latest_change_at)
        }
        VerificationDiffSource::Session(source) => {
            let latest_change_at = latest_session_diff_modified_at(&source.records);
            let review_input = session_diff_review_input(&source.records);
            let (added, removed) = diff_line_counts(&review_input);
            let scope = if path_filters.is_empty() {
                String::new()
            } else {
                format!(" scoped to {}", format_verify_path_filters(path_filters))
            };
            let mut label = format!(
                "session diff fallback from {}{} with {} record(s), +{} -{} changed line(s)",
                source.session.id(),
                scope,
                source.records.len(),
                added,
                removed
            );
            if let Some(note) = source.note {
                label.push_str(&format!(" ({note})"));
            }
            (label, Some(review_input), latest_change_at)
        }
        VerificationDiffSource::None {
            git_available,
            detail,
        } => {
            let mut label = if git_available {
                "none: no local Git diff and no session diff records found".to_string()
            } else {
                "none: Git diff unavailable and no session diff records found".to_string()
            };
            if let Some(detail) = detail {
                label.push_str(&format!(" ({detail})"));
            }
            (label, None, None)
        }
    }
}

fn format_handoff_report_markdown(report: &str) -> String {
    let mut lines = vec!["# deepcli Handoff".to_string()];
    for line in report.lines() {
        if line == "handoff report" || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            if let Some(section) = line.strip_suffix(':') {
                if let Some(title) = handoff_markdown_section_title(section) {
                    lines.push(String::new());
                    lines.push(format!("## {title}"));
                    continue;
                }
            }
        }
        if let Some(item) = line.strip_prefix("  - ") {
            lines.push(format!("- {item}"));
        } else if let Some(item) = line.strip_prefix("- ") {
            lines.push(format!("- {item}"));
        } else if let Some(item) = line.strip_prefix("  ") {
            lines.push(item.to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn handoff_markdown_section_title(section: &str) -> Option<&'static str> {
    match section {
        "summary" => Some("Summary"),
        "changed files" => Some("Changed Files"),
        "review" => Some("Review"),
        "tests" => Some("Tests"),
        "environment" => Some("Environment"),
        "risks and blockers" => Some("Risks and Blockers"),
        "next actions" => Some("Next Actions"),
        _ => None,
    }
}

pub(crate) fn format_handoff_report_pr_description(report: &str) -> String {
    let blockers = handoff_report_blockers(report);
    let mut lines = vec![
        "<!-- generated by deepcli handoff --pr -->".to_string(),
        "## Summary".to_string(),
    ];
    append_pr_section_items(
        &mut lines,
        &handoff_report_section_lines(report, "summary:"),
        "No summary evidence available.",
    );

    append_pr_section(
        &mut lines,
        "Changes",
        &handoff_report_section_lines(report, "changed files:"),
        "No changed files were detected.",
    );
    append_pr_section(
        &mut lines,
        "Test Plan",
        &handoff_report_section_lines(report, "tests:"),
        "No test evidence was recorded.",
    );
    append_pr_section(
        &mut lines,
        "Environment",
        &handoff_report_section_lines(report, "environment:"),
        "No environment evidence was requested.",
    );

    lines.push(String::new());
    lines.push("## Risks and Blockers".to_string());
    if blockers.is_empty() {
        lines.push("- No blockers detected by deepcli handoff.".to_string());
    } else {
        lines.extend(blockers.iter().map(|item| format!("- BLOCKER: {item}")));
    }

    append_pr_section(
        &mut lines,
        "Next Actions",
        &handoff_report_section_lines(report, "next actions:"),
        "No next actions were suggested.",
    );

    lines.push(String::new());
    lines.push("## Checklist".to_string());
    lines.push("- [ ] Review the changed files and generated diff summary".to_string());
    lines.push("- [ ] Confirm the test evidence is sufficient for this change".to_string());
    if blockers.is_empty() {
        lines.push("- [ ] Complete human review before merge".to_string());
    } else {
        lines.push("- [ ] Resolve all blockers before merge".to_string());
    }

    lines.join("\n")
}

fn append_pr_section(lines: &mut Vec<String>, title: &str, section_lines: &[String], empty: &str) {
    lines.push(String::new());
    lines.push(format!("## {title}"));
    append_pr_section_items(lines, section_lines, empty);
}

fn append_pr_section_items(lines: &mut Vec<String>, section_lines: &[String], empty: &str) {
    let mut appended = false;
    for line in section_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            lines.push(format!("- {item}"));
        } else {
            lines.push(format!("- {trimmed}"));
        }
        appended = true;
    }
    if !appended {
        lines.push(format!("- {empty}"));
    }
}

fn handoff_report_section_lines(report: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in report.lines() {
        if line == section {
            in_section = true;
            continue;
        }
        if in_section && !line.starts_with(' ') && line.ends_with(':') {
            break;
        }
        if in_section {
            lines.push(line.to_string());
        }
    }
    lines
}

pub(crate) fn format_handoff_report_json(
    report: &str,
    environment_checks: &[VerificationEnvironmentCheck],
) -> Result<String> {
    let blockers = handoff_report_blockers(report);
    let next_actions = handoff_report_next_actions(report);
    let checklist = delivery_action_checklist(&next_actions);
    let value = json!({
        "schema": schema_ids::HANDOFF_V1,
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
        "checklist": checklist,
        "workspace": handoff_report_prefixed_value(report, "- workspace: "),
        "session": handoff_report_prefixed_value(report, "- session: "),
        "gitStatus": handoff_report_prefixed_value(report, "- git: "),
        "scope": handoff_report_scope_paths(report),
        "diffSource": handoff_report_prefixed_value(report, "- diff: "),
        "environment": verification_environment_json(environment_checks),
        "report": report,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn handoff_report_blockers(report: &str) -> Vec<String> {
    let mut in_blockers = false;
    let mut blockers = Vec::new();
    for line in report.lines() {
        if line == "risks and blockers:" {
            in_blockers = true;
            continue;
        }
        if in_blockers {
            if let Some(item) = line.strip_prefix("  - ") {
                blockers.push(item.to_string());
                continue;
            }
            if line == "next actions:" {
                break;
            }
        }
    }
    blockers
}

fn handoff_report_next_actions(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("  - ") {
                actions.extend(report_next_action_commands(item));
            }
        }
    }
    dedup_preserve_order(actions)
}

fn handoff_report_prefixed_value(report: &str, prefix: &str) -> Option<String> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn handoff_report_scope_paths(report: &str) -> Vec<String> {
    handoff_report_prefixed_value(report, "- scope: paths=")
        .map(|paths| {
            paths
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn blocker_needs_fresh_test_evidence(item: &str) -> bool {
    item.contains("test runs")
        || item.contains("test run failed")
        || item.contains("strong passing test evidence")
        || item.contains("weak test evidence")
}

fn verification_report_has_blockers(report: &str) -> bool {
    !verification_report_blockers(report).is_empty()
}

fn verification_output_has_blockers(output: &str, json_output: bool) -> bool {
    if json_output {
        return serde_json::from_str::<Value>(output)
            .ok()
            .and_then(|value| value.get("hasBlockers").and_then(Value::as_bool))
            .unwrap_or(true);
    }
    verification_report_has_blockers(output)
}

pub(crate) fn format_verification_report_json(
    report: &str,
    environment_checks: &[VerificationEnvironmentCheck],
) -> Result<String> {
    let blockers = verification_report_blockers(report);
    let next_actions = verification_report_next_actions(report);
    let checklist = delivery_action_checklist(&next_actions);
    let value = json!({
        "schema": schema_ids::VERIFY_V1,
        "status": if blockers.is_empty() { "ok" } else { "blocked" },
        "hasBlockers": !blockers.is_empty(),
        "blockers": blockers,
        "nextActions": next_actions,
        "checklist": checklist,
        "workspace": verification_report_prefixed_value(report, "workspace: "),
        "session": verification_report_prefixed_value(report, "session: "),
        "gitStatus": verification_report_prefixed_value(report, "git status: "),
        "scope": verification_report_scope_paths(report),
        "diffSource": verification_report_prefixed_value(report, "diff source: "),
        "environment": verification_environment_json(environment_checks),
        "report": report,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn delivery_action_checklist(actions: &[String]) -> Vec<Value> {
    actions
        .iter()
        .enumerate()
        .map(|(index, command)| {
            json!({
                "step": index + 1,
                "label": delivery_action_label(command),
                "command": command,
            })
        })
        .collect()
}

fn delivery_action_label(command: &str) -> &'static str {
    if command.starts_with("deepcli verify --test-command") {
        "Record cargo test evidence"
    } else if command.starts_with("deepcli verify --run-tests") {
        "Run discovered tests"
    } else if command.starts_with("deepcli verify --env-check docker") {
        "Verify Docker environment"
    } else if command.starts_with("deepcli verify --env-check compiler") {
        "Verify compiler environment"
    } else if command.starts_with("deepcli handoff --env-check docker") {
        "Prepare handoff with Docker evidence"
    } else if command.starts_with("deepcli handoff --env-check compiler") {
        "Prepare handoff with compiler evidence"
    } else if command.starts_with("deepcli handoff") {
        "Prepare handoff report"
    } else if command.starts_with("deepcli session diffs") {
        "Inspect session diffs"
    } else if command.starts_with("deepcli review") {
        "Review current diff"
    } else if command.starts_with("deepcli diff --stat") {
        "Review diff summary"
    } else if command.starts_with("deepcli diff") {
        "Review current diff"
    } else if command.starts_with("git status") {
        "Inspect Git status"
    } else if command.starts_with("cargo test") {
        "Run cargo test"
    } else {
        generic_recipe_command_label(command)
    }
}

fn verification_report_blockers(report: &str) -> Vec<String> {
    let mut in_blockers = false;
    let mut blockers = Vec::new();
    for line in report.lines() {
        if line == "blockers:" {
            in_blockers = true;
            continue;
        }
        if in_blockers {
            if let Some(item) = line.strip_prefix("- ") {
                blockers.push(item.to_string());
                continue;
            }
            if line == "next actions:" {
                break;
            }
        }
    }
    blockers
}

fn verification_report_next_actions(report: &str) -> Vec<String> {
    let mut in_next_actions = false;
    let mut actions = Vec::new();
    for line in report.lines() {
        if line == "next actions:" {
            in_next_actions = true;
            continue;
        }
        if in_next_actions {
            if let Some(item) = line.strip_prefix("- ") {
                actions.extend(report_next_action_commands(item));
            }
        }
    }
    dedup_preserve_order(actions)
}

fn report_next_action_commands(item: &str) -> Vec<String> {
    let quoted = backtick_segments(item);
    let candidates = if quoted.is_empty() {
        vec![item.trim().to_string()]
    } else {
        quoted
    };
    candidates
        .into_iter()
        .filter_map(|candidate| normalize_report_next_action_command(&candidate))
        .collect()
}

fn backtick_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_segment = false;
    for ch in text.chars() {
        if ch == '`' {
            if in_segment {
                let value = current.trim();
                if !value.is_empty() {
                    segments.push(value.to_string());
                }
                current.clear();
                in_segment = false;
            } else {
                in_segment = true;
            }
            continue;
        }
        if in_segment {
            current.push(ch);
        }
    }
    segments
}

fn normalize_report_next_action_command(raw: &str) -> Option<String> {
    let command = slash_to_deepcli_command(raw.trim());
    if command.is_empty() || command.contains('<') {
        return None;
    }
    if command.starts_with("deepcli ")
        || command.starts_with("cargo ")
        || command.starts_with("git ")
    {
        Some(command)
    } else {
        None
    }
}

fn verification_report_prefixed_value(report: &str, prefix: &str) -> Option<String> {
    report
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::to_string))
}

fn verification_report_scope_paths(report: &str) -> Vec<String> {
    verification_report_prefixed_value(report, "scope: paths=")
        .map(|paths| {
            paths
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReviewRiskSummary {
    pub(crate) high_findings: usize,
    pub(crate) medium_findings: usize,
}

pub(crate) fn review_risk_summary_from_report(report: &str) -> ReviewRiskSummary {
    #[derive(Clone, Copy)]
    enum Section {
        High,
        Medium,
        Other,
    }

    let mut section = Section::Other;
    let mut summary = ReviewRiskSummary::default();
    for line in report.lines() {
        match line {
            "high:" => section = Section::High,
            "medium:" => section = Section::Medium,
            "low:" | "worktree:" => section = Section::Other,
            value if value.starts_with("- ") => match section {
                Section::High => summary.high_findings += 1,
                Section::Medium => summary.medium_findings += 1,
                Section::Other => {}
            },
            _ => {}
        }
    }
    summary
}

fn append_verification_test_run(lines: &mut Vec<String>, test_run: &VerificationTestRun) {
    match test_run {
        VerificationTestRun::NotRequested => {}
        VerificationTestRun::Completed {
            command,
            passed,
            exit_code,
            stdout,
            stderr,
        } => {
            lines.push(format!(
                "  requested test run: [{}] exit={exit_code:?} command={command}",
                if *passed { "passed" } else { "failed" }
            ));
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            if !detail.is_empty() {
                lines.push(format!(
                    "  requested output: {}",
                    compact_text_line(detail, 240)
                ));
            }
        }
        VerificationTestRun::Error(error) => {
            lines.push(format!(
                "  requested test run: error {}",
                compact_text_line(error, 240)
            ));
        }
    }
}

fn append_verification_environment(
    lines: &mut Vec<String>,
    blockers: &mut Vec<String>,
    checks: &[VerificationEnvironmentCheck],
    empty_hint: &str,
) {
    if checks.is_empty() {
        lines.push(empty_hint.to_string());
        return;
    }

    for check in checks {
        match check {
            VerificationEnvironmentCheck::Completed {
                target,
                report,
                text,
            } => {
                lines.push(format!(
                    "  {target}: [{}] ready={} recommended={}",
                    environment_status(report.ready),
                    report.ready,
                    report
                        .recommended_action
                        .as_deref()
                        .map(with_smoke)
                        .unwrap_or_else(|| "<none>".to_string())
                ));
                let missing = report
                    .checks
                    .iter()
                    .filter(|check| !check.available)
                    .map(|check| check.name.as_str())
                    .collect::<Vec<_>>();
                if missing.is_empty() {
                    lines.push("  checks: all available".to_string());
                } else {
                    lines.push(format!("  missing checks: {}", missing.join(", ")));
                }
                let detail = first_line(text);
                if !detail.is_empty() {
                    lines.push(format!(
                        "  environment report: {}",
                        compact_text_line(detail, 240)
                    ));
                }
                if !report.ready {
                    let action = report
                        .recommended_action
                        .as_deref()
                        .map(with_smoke)
                        .unwrap_or_else(|| format!("/env plan {target} --smoke"));
                    blockers.push(format!(
                        "environment `{target}` is not ready; run `{}`",
                        action
                    ));
                }
            }
            VerificationEnvironmentCheck::Error { target, error } => {
                lines.push(format!(
                    "  {target}: error {}",
                    compact_text_line(error, 240)
                ));
                blockers.push(format!("environment `{target}` check failed"));
            }
        }
    }
}

fn verification_environment_json(checks: &[VerificationEnvironmentCheck]) -> Value {
    if checks.is_empty() {
        return json!({
            "requested": false,
            "targets": [],
        });
    }

    json!({
        "requested": true,
        "targets": checks.iter().map(verification_environment_check_json).collect::<Vec<_>>(),
    })
}

fn verification_environment_check_json(check: &VerificationEnvironmentCheck) -> Value {
    match check {
        VerificationEnvironmentCheck::Completed {
            target,
            report,
            text,
        } => json!({
            "target": target,
            "status": environment_status(report.ready),
            "ready": report.ready,
            "checks": environment_checks_json(report),
        "recommendedAction": report.recommended_action.as_deref().map(|action| redact_sensitive_text(&with_smoke(action))),
            "report": redact_sensitive_text(text),
        }),
        VerificationEnvironmentCheck::Error { target, error } => json!({
            "target": target,
            "status": "error",
            "ready": false,
            "error": redact_sensitive_text(error),
        }),
    }
}

fn format_git_status_summary(available: bool, status: &str, detail: Option<String>) -> String {
    if !available {
        return format!(
            "git status: unavailable{}",
            detail
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        );
    }
    let changed = status
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let untracked = status
        .lines()
        .filter(|line| line.starts_with("?? "))
        .count();
    if changed == 0 {
        "git status: clean".to_string()
    } else {
        format!("git status: {changed} changed path(s), {untracked} untracked")
    }
}

fn diff_line_counts(diff: &str) -> (usize, usize) {
    let added = diff
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .count();
    let removed = diff
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count();
    (added, removed)
}

fn session_verification_blockers(session: &Session) -> Result<Vec<String>> {
    let mut blockers = Vec::new();
    if matches!(
        session.metadata.state,
        SessionState::AwaitingApproval | SessionState::Failed | SessionState::Paused
    ) {
        blockers.push(format!("session state is {:?}", session.metadata.state));
    }
    let pending_approvals = session
        .load_approval_requests()?
        .iter()
        .filter(|item| item.status == ApprovalStatus::Pending)
        .count();
    if pending_approvals > 0 {
        blockers.push(format!("{pending_approvals} pending approval request(s)"));
    }
    let open_questions = session
        .load_side_questions()?
        .iter()
        .filter(|item| item.status == SideQuestionStatus::Open)
        .count();
    if open_questions > 0 {
        blockers.push(format!("{open_questions} open by-the-way question(s)"));
    }
    let failed_tools = session
        .load_tool_calls()?
        .iter()
        .filter(|item| is_failed_or_denied_tool_call(item))
        .count();
    if failed_tools > 0 {
        blockers.push(format!("{failed_tools} failed or denied tool call(s)"));
    }
    let failed_tests = session
        .load_test_runs()?
        .iter()
        .filter(|item| !item.passed)
        .count();
    if failed_tests > 0 {
        blockers.push(format!("{failed_tests} failed test run(s)"));
    }
    if let Some(plan) = session.load_plan()? {
        let failed_steps = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStepStatus::Failed)
            .count();
        let incomplete_steps = plan
            .steps
            .iter()
            .filter(|step| {
                matches!(
                    step.status,
                    PlanStepStatus::Pending | PlanStepStatus::InProgress
                )
            })
            .count();
        if failed_steps > 0 {
            blockers.push(format!("{failed_steps} failed plan step(s)"));
        }
        if incomplete_steps > 0 {
            blockers.push(format!("{incomplete_steps} incomplete plan step(s)"));
        }
    }
    Ok(blockers)
}

fn verification_next_actions(
    session: Option<&Session>,
    has_diff: bool,
    no_blockers: bool,
    test_run_requested: bool,
    needs_fresh_test_evidence: bool,
    path_filters: &[String],
    environment_checks: &[VerificationEnvironmentCheck],
) -> Vec<String> {
    let session_short = session.map(|session| short_id(&session.id()));
    let mut actions = Vec::new();
    if let Some(short) = session_short.as_deref() {
        if !no_blockers {
            actions.push(format!("- inspect recovery plan: `/session next {short}`"));
        }
        actions.push(format!(
            "- inspect recorded tests: `/session tests --limit 5 {short}`"
        ));
        actions.push(format!(
            "- inspect session usage and trace: `/usage {short}` and `/trace --limit 30 {short}`"
        ));
    }
    if has_diff {
        let scope_args = format_path_scope_args(path_filters);
        actions.push(format!("- review changes: `/review{scope_args}`"));
        actions.push(format!(
            "- inspect diff summary: `/diff --stat{scope_args}`"
        ));
        actions.push(format!(
            "- inspect limited diff: `/diff --limit 200{scope_args}`"
        ));
    }
    if needs_fresh_test_evidence {
        let scope_args = format_path_scope_args(path_filters);
        actions.push(format!(
            "- add strong test evidence: `/verify --run-tests{scope_args}` or `/verify --test-command 'cargo test'{scope_args}`"
        ));
    } else if !test_run_requested {
        actions.push(
            "- include a fresh test run in this report: `/verify --run-tests` or `/verify --test-command 'cargo test'`"
                .to_string(),
        );
    }
    if environment_checks.is_empty() {
        actions.push(
            "- include environment readiness if Docker/compiler matters: `/verify --env-check docker` or `/verify --env-check compiler`"
                .to_string(),
        );
    } else {
        for check in environment_checks {
            match check {
                VerificationEnvironmentCheck::Completed { target, report, .. } if !report.ready => {
                    if let Some(action) = &report.recommended_action {
                        actions.push(format!(
                            "- repair environment `{target}`: `{}`",
                            with_smoke(action)
                        ));
                    } else {
                        actions.push(format!(
                            "- inspect environment `{target}`: `/env plan {target} --smoke --json`"
                        ));
                    }
                }
                VerificationEnvironmentCheck::Error { target, .. } => actions.push(format!(
                    "- inspect environment `{target}`: `/env plan {target} --smoke --json`"
                )),
                _ => {}
            }
        }
    }
    if no_blockers && has_diff {
        actions.push("- if the report matches expectations, prepare handoff with `/git message` or commit through `/git commit <message>`".to_string());
    } else if !has_diff {
        actions.push("- no diff evidence found; run the task or inspect `/session diffs` before accepting implementation work".to_string());
    }
    actions
}

fn append_handoff_environment_next_actions(
    lines: &mut Vec<String>,
    environment_checks: &[VerificationEnvironmentCheck],
) {
    if environment_checks.is_empty() {
        lines.push(
            "  - include environment readiness if Docker/compiler matters: `/handoff --env-check docker` or `/handoff --env-check compiler`"
                .to_string(),
        );
        return;
    }
    for check in environment_checks {
        match check {
            VerificationEnvironmentCheck::Completed { target, report, .. } if !report.ready => {
                if let Some(action) = &report.recommended_action {
                    lines.push(format!(
                        "  - repair environment `{target}`: `{}`",
                        with_smoke(action)
                    ));
                } else {
                    lines.push(format!(
                        "  - inspect environment `{target}`: `/env plan {target} --smoke --json`"
                    ));
                }
            }
            VerificationEnvironmentCheck::Error { target, .. } => {
                lines.push(format!(
                    "  - inspect environment `{target}`: `/env plan {target} --smoke --json`"
                ));
            }
            _ => {}
        }
    }
}

fn command_succeeded(raw: &Value) -> bool {
    command_exit_code(raw).is_none_or(|code| code == 0)
}

fn command_exit_code(raw: &Value) -> Option<i32> {
    raw.get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn command_stderr(raw: &Value) -> Option<&str> {
    raw.get("stderr").and_then(Value::as_str)
}

fn command_failure_detail(output: &crate::tools::ToolExecution) -> Option<String> {
    let detail = command_stderr(&output.raw)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&output.content);
    let detail = first_line(detail).trim();
    if detail.is_empty() {
        None
    } else {
        Some(detail.to_string())
    }
}

fn resolve_scoped_session_diff_source(
    workspace: &Path,
    current: Option<&str>,
    limit: usize,
    filters: &[String],
) -> Result<Option<SessionDiffSource>> {
    Ok(resolve_session_diff_source(workspace, current, limit)?
        .and_then(|source| filter_session_diff_source_by_paths(source, filters)))
}

fn filter_session_diff_source_by_paths(
    mut source: SessionDiffSource,
    filters: &[String],
) -> Option<SessionDiffSource> {
    if filters.is_empty() {
        return Some(source);
    }
    source
        .records
        .retain(|record| session_diff_record_matches_filters(record, filters));
    if source.records.is_empty() {
        None
    } else {
        Some(source)
    }
}

fn session_diff_record_matches_filters(record: &SessionDiffRecord, filters: &[String]) -> bool {
    if path_matches_verify_filters(&record.name, filters) {
        return true;
    }
    let record_name = record.name.replace('\\', "/");
    if filters.iter().any(|filter| {
        let sanitized = filter
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        record_name.contains(&sanitized)
    }) {
        return true;
    }
    !filter_diff_by_paths(&record.content, filters)
        .trim()
        .is_empty()
}

fn resolve_session_diff_source(
    workspace: &Path,
    current: Option<&str>,
    limit: usize,
) -> Result<Option<SessionDiffSource>> {
    let store = SessionStore::new(workspace);
    if let Some(id) = current {
        let current_session = store.load(id)?;
        let records = load_non_empty_session_diffs(&current_session, limit)?;
        if !records.is_empty() {
            return Ok(Some(SessionDiffSource {
                session: current_session,
                note: None,
                records,
            }));
        }
        if let Some(mut source) = latest_session_with_diffs(&store, Some(id), limit)? {
            source.note = Some(format!(
                "latest session with {}; current session {id} had none",
                session_fallback_label(SessionFallbackKind::Diffs)
            ));
            return Ok(Some(source));
        }
        return Ok(None);
    }

    latest_session_with_diffs(&store, None, limit).map(|option| {
        option.map(|mut source| {
            source.note = Some("latest session with diff records; no current session".to_string());
            source
        })
    })
}

fn latest_session_with_diffs(
    store: &SessionStore,
    skip_id: Option<&str>,
    limit: usize,
) -> Result<Option<SessionDiffSource>> {
    for metadata in store.list()? {
        let id = metadata.id.to_string();
        if skip_id.is_some_and(|skip| skip == id) {
            continue;
        }
        let session = store.load(&id)?;
        let records = load_non_empty_session_diffs(&session, limit)?;
        if !records.is_empty() {
            return Ok(Some(SessionDiffSource {
                session,
                note: None,
                records,
            }));
        }
    }
    Ok(None)
}

fn load_non_empty_session_diffs(session: &Session, limit: usize) -> Result<Vec<SessionDiffRecord>> {
    Ok(session
        .load_recent_diffs(limit)?
        .into_iter()
        .filter(|record| !record.content.trim().is_empty())
        .collect())
}

fn format_session_diff_fallback(source: &SessionDiffSource) -> String {
    let mut output = session_diff_fallback_header(source);
    output.push('\n');
    output.push_str(&format_session_diffs(
        &source.records,
        SESSION_DIFF_FALLBACK_LIMIT,
    ));
    output
}

fn session_diff_fallback_header(source: &SessionDiffSource) -> String {
    let mut output = format!("session diff fallback: session {}", source.session.id());
    if let Some(title) = source.session.metadata.title.as_deref() {
        output.push_str(&format!(" ({title})"));
    }
    if let Some(note) = source.note.as_deref() {
        output.push_str(&format!("\nnote: {note}"));
    }
    output
}

fn session_diff_review_input(records: &[SessionDiffRecord]) -> String {
    records
        .iter()
        .map(|record| {
            format!(
                "diff --session {}\n{}",
                session_diff_record_display_path(record),
                record.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn session_diff_record_display_path(record: &SessionDiffRecord) -> String {
    record
        .content
        .lines()
        .find_map(diff_section_path_from_line)
        .unwrap_or_else(|| {
            record
                .name
                .split_once("Z-")
                .map(|(_, rest)| rest)
                .filter(|rest| !rest.is_empty())
                .unwrap_or(&record.name)
                .to_string()
        })
}

pub(crate) fn review_diff(diff: &str) -> String {
    if diff.trim().is_empty() {
        return "auto-reviewer: no local diff to review".to_string();
    }

    let mut high = ReviewFindings::default();
    let mut medium = ReviewFindings::default();
    let mut low = ReviewFindings::default();
    let (added_lines, removed_lines) = diff_line_counts(diff);
    let mut current_path: Option<String> = None;
    let mut in_probable_test_context = false;

    for (index, line) in diff.lines().enumerate() {
        let line_number = index + 1;
        if let Some(path) = review_path_from_diff_line(line) {
            current_path = Some(path);
            in_probable_test_context = false;
        }
        if is_review_test_marker_line(line) {
            in_probable_test_context = true;
        }
        let path = current_path.as_deref();
        if is_sensitive_review_line(line, path, in_probable_test_context) {
            high.add(
                "added line appears to contain sensitive material",
                Some(review_finding_example(line_number, line)),
            );
        }
        if is_dangerous_command_review_line(line, path, in_probable_test_context) {
            high.add(
                "diff contains a dangerous command pattern",
                Some(review_finding_example(line_number, line)),
            );
        }
        if line.starts_with("diff --git") && review_path_touches_credentials(path) {
            high.add(
                "diff touches local credentials path",
                Some(review_finding_example(line_number, line)),
            );
        }
        if is_panic_prone_review_line(line, path, in_probable_test_context) {
            medium.add(
                "added Rust panic-prone call; confirm it is acceptable",
                Some(review_finding_example(line_number, line)),
            );
        }
    }

    if added_lines + removed_lines > 500 {
        medium.add("large diff; consider splitting review scope", None);
    }
    if high.is_empty() && medium.is_empty() {
        low.add("no obvious high-risk pattern found", None);
    }

    let mut report = vec![
        "auto-reviewer report".to_string(),
        format!("changed lines: +{added_lines} -{removed_lines}"),
    ];
    append_findings(&mut report, "high", &high);
    append_findings(&mut report, "medium", &medium);
    append_findings(&mut report, "low", &low);
    report.join("\n")
}

#[derive(Debug, Default)]
struct ReviewFindings {
    items: Vec<ReviewFinding>,
}

#[derive(Debug)]
struct ReviewFinding {
    message: &'static str,
    count: usize,
    examples: Vec<String>,
}

impl ReviewFindings {
    fn add(&mut self, message: &'static str, example: Option<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.message == message) {
            item.count += 1;
            if let Some(example) = example {
                if item.examples.len() < 3 {
                    item.examples.push(example);
                }
            }
            return;
        }

        self.items.push(ReviewFinding {
            message,
            count: 1,
            examples: example.into_iter().collect(),
        });
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn is_added_diff_line(line: &str) -> bool {
    line.starts_with('+') && !line.starts_with("+++")
}

fn review_path_from_diff_line(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        let mut parts = rest.split_whitespace();
        let first = parts.next();
        let second = parts.next();
        return second
            .and_then(normalize_review_diff_path)
            .or_else(|| first.and_then(normalize_review_diff_path));
    }
    if let Some(rest) = line.strip_prefix("diff --session ") {
        let path = rest.trim();
        return (!path.is_empty()).then(|| path.to_string());
    }
    if let Some(rest) = line.strip_prefix("+++ ") {
        return normalize_review_diff_path(rest.trim());
    }
    None
}

fn normalize_review_diff_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('"');
    if trimmed == "/dev/null" || trimmed.is_empty() {
        return None;
    }
    Some(
        trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed)
            .to_string(),
    )
}

fn review_path_touches_credentials(path: Option<&str>) -> bool {
    path.is_some_and(|path| {
        let normalized = path.replace('\\', "/");
        normalized == ".deepcli/credentials" || normalized.ends_with("/.deepcli/credentials")
    })
}

fn is_review_test_or_doc_path(path: Option<&str>) -> bool {
    path.is_some_and(|path| {
        let normalized = path.replace('\\', "/");
        normalized.starts_with("tests/")
            || normalized.contains("/tests/")
            || normalized.starts_with("docs/")
            || normalized.contains("/docs/")
            || normalized.ends_with("_test.rs")
            || normalized.ends_with(".md")
            || normalized.ends_with(".rst")
            || normalized.ends_with(".txt")
    })
}

fn is_review_test_marker_line(line: &str) -> bool {
    let text = diff_line_text(line).trim();
    text.starts_with("#[test]") || text.starts_with("#[tokio::test]") || text.contains("mod tests")
}

fn diff_line_text(line: &str) -> &str {
    match line.as_bytes().first() {
        Some(b'+' | b'-' | b' ') => &line[1..],
        _ => line,
    }
}

fn is_sensitive_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//") || text.starts_with('#') || text.starts_with('*') {
        return false;
    }
    if !looks_sensitive(text) {
        return false;
    }
    if is_sensitive_review_detector_source_line(text) {
        return false;
    }
    if has_explicit_secret_review_marker(text) {
        return true;
    }
    !is_safe_sensitive_review_source_line(text)
}

fn is_sensitive_review_detector_source_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let defines_secret_marker = lower.contains("lower.contains(")
        && (lower.contains("sk-")
            || lower.contains("bearer ")
            || lower.contains("-----begin private key-----"));
    let defines_api_key_rule = (lower.contains("lower.contains(")
        || lower.contains("lower.starts_with("))
        && (lower.contains("api_key") || lower.contains("apikey"));
    let defines_api_key_trim_rule = lower.contains("trim_end_matches") && lower.contains("api_key");
    lower.contains("has_explicit_secret_review_marker")
        || lower.contains("secret_markers")
        || lower.contains("secret_value_markers")
        || lower.contains("sensitive_header_markers")
        || lower.contains("has_secret_value_marker")
        || lower.contains("has_sensitive_header_marker")
        || lower.contains("contains_sk_secret_marker")
        || lower.contains("privacy_has_secret_value_marker")
        || lower.contains("mentions_api_key")
        || lower.contains("defines_api_key_rule")
        || lower.contains("defines_api_key_trim_rule")
        || lower.contains("safe_api_key_source_reference")
        || defines_secret_marker
        || defines_api_key_rule
        || defines_api_key_trim_rule
}

fn has_explicit_secret_review_marker(text: &str) -> bool {
    privacy_has_secret_value_marker(text)
}

fn is_safe_sensitive_review_source_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("<redacted>")
        || lower.contains("redacted")
        || lower.contains("secret_review_marker")
        || lower.contains("secret_marker")
        || lower.contains("secret_markers")
        || lower.contains("looks_sensitive")
        || lower.contains("redact_sensitive")
        || lower.contains("is_sensitive_key")
        || lower.contains("sensitive material")
        || lower.contains("look like secrets")
    {
        return true;
    }
    if lower.contains("authorization: {}")
        || (lower.contains('$') && lower.contains("_api_key"))
        || (lower.contains("provider api keys") && lower.contains("_api_key"))
        || (lower.contains("format!(") && lower.contains("_api_key"))
        || lower.contains("api_key={}")
        || lower.contains("apikey redacted")
        || lower.contains("apikey must not be empty")
        || lower.contains("apikey for provider")
        || lower.contains("api_key missing")
        || lower.contains("api_key=missing")
        || lower.contains("api_key=configured")
    {
        return true;
    }
    let mentions_api_key = lower.contains("api_key") || lower.contains("apikey");
    let safe_api_key_source_reference = lower.contains(".api_key")
        || lower.contains(" api_key:")
        || lower.starts_with("api_key:")
        || lower.contains("api_key: string")
        || lower.contains("api_key: option")
        || lower.contains("let api_key =")
        || lower.contains("let mut api_key")
        || lower.contains("file_api_key")
        || lower.contains("read_api_key")
        || lower.contains("set_credentials_api_key")
        || lower.contains("store_provider_api_key")
        || lower.contains("provider_api_key")
        || lower.contains("provider_env_key")
        || lower.contains("api_key.trim")
        || lower.contains("api_key.pop")
        || lower.contains("api_key.push")
        || lower.contains("api_key.is_empty")
        || lower.contains("&mut api_key")
        || lower.contains("ok(api_key)")
        || lower.trim_end_matches(',') == "api_key";
    if mentions_api_key && safe_api_key_source_reference {
        return true;
    }
    false
}

fn is_dangerous_command_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//") || text.starts_with('#') || text.starts_with('*') {
        return false;
    }
    if is_review_detector_literal_line(text) {
        return false;
    }
    text.contains("rm -rf") || text.contains("git reset --hard")
}

fn is_review_detector_literal_line(text: &str) -> bool {
    (text.contains("rm -rf") || text.contains("git reset --hard")) && text.contains(".contains(")
}

fn is_panic_prone_review_line(
    line: &str,
    path: Option<&str>,
    in_probable_test_context: bool,
) -> bool {
    if !is_added_diff_line(line) || is_review_test_or_doc_path(path) || in_probable_test_context {
        return false;
    }
    let text = diff_line_text(line).trim();
    if text.starts_with("//")
        || text.starts_with("assert!")
        || text.starts_with("assert_eq!")
        || text.starts_with("assert_ne!")
        || is_panic_review_detector_source_line(text)
        || is_documented_invariant_expect_line(text)
    {
        return false;
    }
    text.contains("unwrap()") || text.contains("expect(")
}

fn is_panic_review_detector_source_line(text: &str) -> bool {
    text.contains("text.contains(\"unwrap()\")")
        || text.contains("text.contains(\"expect(\")")
        || text.contains("is_documented_invariant_expect_line")
}

fn is_documented_invariant_expect_line(text: &str) -> bool {
    if !text.contains("expect(") {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    lower.contains("checked")
        || lower.contains("validated")
        || lower.contains("guaranteed")
        || lower.contains("known")
        || lower.contains("already")
        || lower.contains("invariant")
}

fn review_finding_example(line_number: usize, line: &str) -> String {
    let redacted = redact_sensitive_text(line);
    format!("line {line_number}: {}", compact_text_line(&redacted, 180))
}

pub(crate) fn review_worktree(status: &str, diff: &str) -> String {
    let mut report = review_diff(diff);
    let untracked = status
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .collect::<Vec<_>>();
    if !untracked.is_empty() {
        report.push_str("\nworktree:");
        report.push_str(&format!("\n- untracked files: {}", untracked.len()));
        for path in untracked.iter().take(8) {
            report.push_str(&format!("\n  - {path}"));
        }
        if untracked.len() > 8 {
            report.push_str("\n  - ...");
        }
    }
    report
}

fn append_findings(report: &mut Vec<String>, label: &str, findings: &ReviewFindings) {
    if findings.is_empty() {
        return;
    }
    report.push(format!("{label}:"));
    for finding in &findings.items {
        if finding.count == 1 {
            report.push(format!("- {}", finding.message));
        } else {
            report.push(format!(
                "- {} ({} occurrences)",
                finding.message, finding.count
            ));
        }
        for example in &finding.examples {
            report.push(format!("  example: {example}"));
        }
    }
}
