use super::{
    compact_text_line, dedup_preserve_order, format_session_diagnosis, handle_doctor, handle_logs,
    handle_quickstart, handle_session, handle_status, handle_trace, handle_usage, handle_version,
    indent_text, local_action_checklist, parse_positive_usize, prefix_session_note, required_arg,
    resolve_session_for_next_actions, set_command_output_path, workspace_relative_display,
    write_command_output, CommandContext, DEFAULT_SUPPORT_BUNDLE_DIR,
};
use crate::config::AppConfig;
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::session::SessionStore;
use crate::tools::{resolve_workspace_path, ToolExecutor, ToolRegistry};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) async fn handle_diagnose(
    workspace: &Path,
    config: &AppConfig,
    executor: &ToolExecutor,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_diagnose_options(&args, current)?;
    let mut doctor_args = Vec::new();
    if !options.full_environment {
        doctor_args.push("--quick".to_string());
    }
    if options.probe_provider {
        doctor_args.push("--probe-provider".to_string());
    }
    if let Some(provider) = &options.provider {
        doctor_args.push("--provider".to_string());
        doctor_args.push(provider.clone());
    }

    let workspace_health = handle_doctor(
        workspace,
        config,
        executor,
        options.session_id.clone(),
        doctor_args,
    )
    .await?;
    let session_section = format_global_diagnose_session_section(
        workspace,
        options.session_id.as_deref(),
        options.explicit_session,
        options.limit,
    )?;

    let mut lines = vec![
        "deepcli diagnose".to_string(),
        "workspace health:".to_string(),
        indent_text(&workspace_health, "  "),
        "session diagnosis:".to_string(),
        indent_text(&session_section, "  "),
        "quick links:".to_string(),
        "  - first-run guide: `/quickstart`".to_string(),
        "  - fix local setup: `/init --quick`".to_string(),
        "  - full environment check: `/diagnose --full-env`".to_string(),
        "  - online provider probe: `/diagnose --probe-provider`".to_string(),
        "  - session-only diagnosis: `/session diagnose`".to_string(),
    ];
    if options.provider.is_none() && !options.probe_provider {
        lines.push(
            "  - provider-specific probe: `/diagnose --probe-provider --provider <name>`"
                .to_string(),
        );
    }
    let base_report = lines.join("\n");
    let support_bundle = options
        .bundle_dir
        .as_deref()
        .map(|bundle_dir| {
            write_diagnose_support_bundle(DiagnoseSupportBundleInput {
                workspace,
                config,
                executor,
                options: &options,
                workspace_health: &workspace_health,
                session_diagnosis: &session_section,
                report: &base_report,
                raw_dir: bundle_dir,
            })
        })
        .transpose()?;
    let report = if let Some(bundle) = &support_bundle {
        append_diagnose_support_bundle_summary(&base_report, bundle)
    } else {
        base_report
    };
    let output = if options.json_output {
        format_diagnose_report_json(
            workspace,
            &options,
            &workspace_health,
            &session_section,
            &report,
            support_bundle.as_ref(),
        )?
    } else {
        report
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DiagnoseOptions {
    pub(crate) full_environment: bool,
    pub(crate) probe_provider: bool,
    pub(crate) provider: Option<String>,
    pub(crate) limit: usize,
    pub(crate) session_id: Option<String>,
    pub(crate) explicit_session: bool,
    pub(crate) json_output: bool,
    pub(crate) output_path: Option<String>,
    pub(crate) bundle_dir: Option<String>,
}

pub(crate) fn parse_diagnose_options(
    args: &[String],
    current: Option<String>,
) -> Result<DiagnoseOptions> {
    let mut options = DiagnoseOptions {
        full_environment: false,
        probe_provider: false,
        provider: None,
        limit: 5,
        session_id: current,
        explicit_session: false,
        json_output: false,
        output_path: None,
        bundle_dir: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--quick" | "--no-env" => {
                options.full_environment = false;
                index += 1;
            }
            "--full-env" | "--full" => {
                options.full_environment = true;
                index += 1;
            }
            "--probe-provider" => {
                options.probe_provider = true;
                index += 1;
            }
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut options.output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(
                    &mut options.output_path,
                    value.trim_start_matches("--output="),
                )?;
                index += 1;
            }
            "--bundle" => {
                let raw = required_arg(args, index + 1, "bundle dir")?;
                set_command_output_path(&mut options.bundle_dir, raw)?;
                index += 2;
            }
            value if value.starts_with("--bundle=") => {
                set_command_output_path(
                    &mut options.bundle_dir,
                    value.trim_start_matches("--bundle="),
                )?;
                index += 1;
            }
            "--provider" => {
                options.provider = Some(required_arg(args, index + 1, "provider")?.to_string());
                index += 2;
            }
            value if value.starts_with("--provider=") => {
                let provider = value
                    .strip_prefix("--provider=")
                    .expect("prefix checked")
                    .trim();
                if provider.is_empty() {
                    bail!("missing provider");
                }
                options.provider = Some(provider.to_string());
                index += 1;
            }
            "--limit" | "-n" => {
                options.limit =
                    parse_positive_usize(required_arg(args, index + 1, "limit")?, "limit")?
                        .clamp(1, 100);
                index += 2;
            }
            value if value.starts_with("--limit=") => {
                let limit = value
                    .strip_prefix("--limit=")
                    .expect("prefix checked")
                    .trim();
                options.limit = parse_positive_usize(limit, "limit")?.clamp(1, 100);
                index += 1;
            }
            "--current" => {
                if options.explicit_session {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(
                    options
                        .session_id
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("no active session is available"))?,
                );
                options.explicit_session = true;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /diagnose option `{value}`"),
            value => {
                if options.explicit_session {
                    bail!("multiple session ids were provided");
                }
                options.session_id = Some(value.to_string());
                options.explicit_session = true;
                index += 1;
            }
        }
    }

    if options.provider.is_some() && !options.probe_provider {
        bail!("`/diagnose --provider <name>` requires `--probe-provider`");
    }

    Ok(options)
}

fn format_diagnose_report_json(
    workspace: &Path,
    options: &DiagnoseOptions,
    workspace_health: &str,
    session_diagnosis: &str,
    report: &str,
    support_bundle: Option<&DiagnoseSupportBundleResult>,
) -> Result<String> {
    let next_actions = diagnose_next_actions(options);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::DIAGNOSE_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "mode": {
            "fullEnvironment": options.full_environment,
            "probeProvider": options.probe_provider,
            "provider": options.provider.as_deref(),
            "limit": options.limit,
            "explicitSession": options.explicit_session,
            "session": options.session_id.as_deref(),
        },
        "workspaceHealth": workspace_health,
        "sessionDiagnosis": session_diagnosis,
        "supportBundle": support_bundle
            .map(diagnose_support_bundle_json)
            .unwrap_or(Value::Null),
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
        "report": report,
    }))?)
}

#[derive(Debug, Clone)]
struct DiagnoseSupportBundleResult {
    directory: PathBuf,
    manifest_path: PathBuf,
    files: Vec<DiagnoseSupportBundleFile>,
}

#[derive(Debug, Clone)]
struct DiagnoseSupportBundleFile {
    name: String,
    path: String,
    ok: bool,
    bytes: u64,
    error: Option<String>,
}

struct DiagnoseSupportBundleInput<'a> {
    workspace: &'a Path,
    config: &'a AppConfig,
    executor: &'a ToolExecutor,
    options: &'a DiagnoseOptions,
    workspace_health: &'a str,
    session_diagnosis: &'a str,
    report: &'a str,
    raw_dir: &'a str,
}

fn write_diagnose_support_bundle(
    input: DiagnoseSupportBundleInput<'_>,
) -> Result<DiagnoseSupportBundleResult> {
    let workspace = input.workspace;
    let directory = resolve_workspace_path(workspace, input.raw_dir)?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    let mut files = Vec::new();
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "README.txt",
        || {
            Ok(format!(
            "deepcli support bundle\nworkspace: {}\ncreated_at: {}\n\nThis bundle is generated by `/diagnose --bundle`.\nArtifacts are redacted and workspace-contained.\n",
            workspace.display(),
            Utc::now().to_rfc3339()
        ))
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "issue.md",
        || {
            Ok(format_diagnose_issue_template(
                workspace,
                &directory,
                input.config,
                input.options,
            ))
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "diagnose.json",
        || {
            format_diagnose_report_json(
                workspace,
                input.options,
                input.workspace_health,
                input.session_diagnosis,
                input.report,
                None,
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "version.json",
        || handle_version(workspace, input.config, vec!["--json".to_string()]),
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "quickstart.json",
        || {
            handle_quickstart(
                workspace,
                input.config,
                input.executor,
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "status.json",
        || {
            let registry = ToolRegistry::mvp();
            handle_status(
                CommandContext {
                    workspace,
                    config: input.config,
                    registry: &registry,
                    executor: input.executor,
                    session_id: input.options.session_id.clone(),
                    provider_override: None,
                    allow_interactive_prompts: true,
                },
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "usage.json",
        || {
            handle_usage(
                workspace,
                input.options.session_id.clone(),
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "trace.json",
        || {
            handle_trace(
                workspace,
                input.options.session_id.clone(),
                vec!["--json".to_string()],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "logs.json",
        || {
            handle_logs(
                workspace,
                vec![
                    "--json".to_string(),
                    "--limit".to_string(),
                    input.options.limit.to_string(),
                ],
            )
        },
        &mut files,
    )?;
    write_diagnose_bundle_artifact(
        workspace,
        &directory,
        "sessions.json",
        || {
            handle_session(
                workspace,
                input.options.session_id.clone(),
                vec![
                    "list".to_string(),
                    "--all".to_string(),
                    "--limit".to_string(),
                    input.options.limit.to_string(),
                    "--json".to_string(),
                ],
            )
        },
        &mut files,
    )?;

    let manifest_path = directory.join("manifest.json");
    let next_actions = diagnose_support_bundle_next_actions(workspace, &directory);
    let manifest = serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SUPPORT_BUNDLE_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "createdAt": Utc::now(),
        "directory": workspace_relative_display(workspace, &directory),
        "mode": {
            "fullEnvironment": input.options.full_environment,
            "probeProvider": input.options.probe_provider,
            "provider": input.options.provider.as_deref(),
            "limit": input.options.limit,
            "explicitSession": input.options.explicit_session,
            "session": input.options.session_id.as_deref(),
        },
        "files": files.iter().map(diagnose_support_bundle_file_json).collect::<Vec<_>>(),
        "checklist": local_action_checklist(&next_actions),
        "nextActions": next_actions,
        "notes": [
            "attach this support bundle when reporting a deepcli issue",
            "start from issue.md when drafting a bug report or support request",
            "inspect diagnose.json first for workspace and session next actions",
            "run the full-environment next action when Docker, compiler, or local environment readiness matters",
        ],
    }))?;
    fs::write(&manifest_path, manifest)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(DiagnoseSupportBundleResult {
        directory,
        manifest_path,
        files,
    })
}

fn write_diagnose_bundle_artifact(
    workspace: &Path,
    directory: &Path,
    name: &str,
    producer: impl FnOnce() -> Result<String>,
    files: &mut Vec<DiagnoseSupportBundleFile>,
) -> Result<()> {
    let (ok, content, error) = match producer() {
        Ok(content) => (true, content, None),
        Err(error) => {
            let error = compact_text_line(&redact_sensitive_text(&error.to_string()), 500);
            let content = serde_json::to_string_pretty(&json!({
                "schema": schema_ids::SUPPORT_BUNDLE_ARTIFACT_V1,
                "status": "error",
                "name": name,
                "error": error,
            }))?;
            (false, content, Some(error))
        }
    };
    let path = directory.join(name);
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    let bytes = fs::metadata(&path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .len();
    files.push(DiagnoseSupportBundleFile {
        name: name.to_string(),
        path: workspace_relative_display(workspace, &path),
        ok,
        bytes,
        error,
    });
    Ok(())
}

fn format_diagnose_issue_template(
    workspace: &Path,
    directory: &Path,
    config: &AppConfig,
    options: &DiagnoseOptions,
) -> String {
    let next_actions = diagnose_next_actions(options);
    let mut lines = vec![
        "# deepcli issue report".to_string(),
        String::new(),
        "## Summary".to_string(),
        "- observed behavior: ".to_string(),
        "- expected behavior: ".to_string(),
        "- impact: ".to_string(),
        String::new(),
        "## Diagnostic Context".to_string(),
        format!("- workspace: {}", workspace.display()),
        format!(
            "- support bundle: {}",
            workspace_relative_display(workspace, directory)
        ),
        format!("- generated at: {}", Utc::now().to_rfc3339()),
        format!("- deepcli version: {}", env!("CARGO_PKG_VERSION")),
        format!("- default provider: {}", config.default_provider),
        format!(
            "- provider turn timeout: {}s",
            config.agent.provider_turn_timeout_seconds
        ),
        format!(
            "- mode: fullEnvironment={} probeProvider={} limit={}",
            options.full_environment, options.probe_provider, options.limit
        ),
        format!(
            "- session: {}",
            options.session_id.as_deref().unwrap_or("<none>")
        ),
        String::new(),
        "## Attachments".to_string(),
        "- manifest.json".to_string(),
        "- version.json".to_string(),
        "- diagnose.json".to_string(),
        "- quickstart.json".to_string(),
        "- status.json".to_string(),
        "- usage.json".to_string(),
        "- trace.json".to_string(),
        "- logs.json".to_string(),
        "- sessions.json".to_string(),
        String::new(),
        "## Next Actions Suggested By deepcli".to_string(),
    ];
    if next_actions.is_empty() {
        lines.push("- inspect diagnose.json for next actions".to_string());
    } else {
        lines.extend(next_actions.into_iter().map(|action| format!("- {action}")));
    }
    lines.extend([
        String::new(),
        "## Notes".to_string(),
        "- Generated artifacts are redacted by deepcli; still review attachments before sharing externally.".to_string(),
        "- Re-run with `/diagnose --full-env --bundle <dir>` if Docker, compiler, or local environment readiness is part of the issue.".to_string(),
    ]);
    lines.join("\n")
}

fn append_diagnose_support_bundle_summary(
    report: &str,
    bundle: &DiagnoseSupportBundleResult,
) -> String {
    format!(
        "{report}\nsupport bundle:\n  path: {}\n  manifest: {}\n  files: {}",
        bundle.directory.display(),
        bundle.manifest_path.display(),
        bundle.files.len()
    )
}

fn diagnose_support_bundle_json(bundle: &DiagnoseSupportBundleResult) -> Value {
    json!({
        "directory": bundle.directory.display().to_string(),
        "manifest": bundle.manifest_path.display().to_string(),
        "files": bundle.files.iter().map(diagnose_support_bundle_file_json).collect::<Vec<_>>(),
    })
}

fn diagnose_support_bundle_file_json(file: &DiagnoseSupportBundleFile) -> Value {
    json!({
        "name": file.name.as_str(),
        "path": file.path.as_str(),
        "ok": file.ok,
        "bytes": file.bytes,
        "error": file.error.as_deref(),
    })
}

fn diagnose_support_bundle_next_actions(workspace: &Path, directory: &Path) -> Vec<String> {
    let bundle_dir = workspace_relative_display(workspace, directory);
    let quoted_bundle_dir = shell_words::quote(&bundle_dir);
    vec![
        "deepcli diagnose --json".to_string(),
        format!("deepcli support {quoted_bundle_dir} --json"),
        format!("deepcli diagnose --full-env --bundle {quoted_bundle_dir} --json"),
    ]
}

fn diagnose_next_actions(options: &DiagnoseOptions) -> Vec<String> {
    let mut actions = vec![
        "deepcli quickstart".to_string(),
        "deepcli init --quick".to_string(),
        if options.full_environment {
            "deepcli diagnose --json".to_string()
        } else {
            "deepcli diagnose --full-env --json".to_string()
        },
    ];
    if options.probe_provider {
        actions.push("deepcli diagnose --json".to_string());
    } else {
        actions.push("deepcli diagnose --probe-provider --json".to_string());
    }
    if let Some(provider) = &options.provider {
        actions.push(format!(
            "deepcli diagnose --probe-provider --provider {} --json",
            shell_words::quote(provider)
        ));
    } else {
        actions.push("deepcli model list --json".to_string());
    }
    actions.push("deepcli session diagnose --json".to_string());
    if options.bundle_dir.is_some() {
        actions.push("deepcli diagnose --json".to_string());
    } else {
        actions.push(format!(
            "deepcli support {} --json",
            shell_words::quote(DEFAULT_SUPPORT_BUNDLE_DIR)
        ));
    }
    dedup_preserve_order(actions)
}

fn format_global_diagnose_session_section(
    workspace: &Path,
    id: Option<&str>,
    explicit: bool,
    limit: usize,
) -> Result<String> {
    let store = SessionStore::new(workspace);
    match resolve_session_for_next_actions(&store, id, explicit) {
        Ok((session, note)) => Ok(prefix_session_note(
            format_session_diagnosis(&session, limit)?,
            &session,
            note,
        )),
        Err(error) if !explicit && id.is_none() => Ok(format!(
            "skipped: {}\nnext: run `deepcli` to start a session, or run `/doctor --quick` for workspace-only checks",
            compact_text_line(&error.to_string(), 200)
        )),
        Err(error) => Err(error),
    }
}
