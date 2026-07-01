use super::*;
use anyhow::{bail, Result};
use serde_json::{json, Value};

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

pub(crate) fn command_succeeded(raw: &Value) -> bool {
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

pub(crate) fn command_failure_detail(output: &crate::tools::ToolExecution) -> Option<String> {
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
