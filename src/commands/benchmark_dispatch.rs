use super::*;
use anyhow::{bail, Result};
use std::path::Path;

pub(crate) fn handle_benchmark(
    workspace: &Path,
    config: &AppConfig,
    registry: &ToolRegistry,
    args: Vec<String>,
) -> Result<String> {
    if benchmark_args_are_scorecard_compatible(&args) {
        return handle_scorecard(workspace, config, registry, args);
    }
    let Some((subcommand, rest)) = args.split_first() else {
        return handle_scorecard(workspace, config, registry, Vec::new());
    };
    match subcommand.as_str() {
        "scorecard" | "rubric" => handle_scorecard(workspace, config, registry, rest.to_vec()),
        "run-suite" | "suite" | "run-all" => {
            handle_benchmark_run_suite(workspace, config, registry, rest)
        }
        "run" | "exec" => handle_benchmark_run(workspace, config, registry, rest),
        "record" | "save" => handle_benchmark_record(workspace, config, registry, rest),
        "presets" | "preset" | "catalog" => handle_benchmark_presets(workspace, rest),
        "status" | "health" | "doctor" => handle_benchmark_status(workspace, rest),
        "gate" | "check" => {
            let mut gate_args = rest.to_vec();
            if !benchmark_status_args_request_failure(&gate_args) {
                gate_args.push("--fail-on-not-ready".to_string());
            }
            handle_benchmark_status(workspace, &gate_args)
        }
        "summary" | "summarize" | "report" => handle_benchmark_summary(workspace, rest),
        "trend" | "trends" | "history" => handle_benchmark_trends(workspace, rest),
        "baseline-template" | "template" | "baseline-init" => {
            handle_benchmark_baseline_template(workspace, rest)
        }
        "compare" | "comparison" | "baseline" => handle_benchmark_compare(workspace, rest),
        "baselines" | "baseline-list" | "baseline-ls" => {
            handle_benchmark_baselines(workspace, rest)
        }
        "list" | "ls" => handle_benchmark_list(workspace, rest),
        "show" | "view" => handle_benchmark_show(workspace, rest),
        "clean" | "cleanup" | "prune" => handle_benchmark_cleanup(workspace, rest),
        "latest" => {
            let mut show_args = vec!["latest".to_string()];
            show_args.extend(rest.iter().cloned());
            handle_benchmark_show(workspace, &show_args)
        }
        value => bail!(
            "unknown /benchmark subcommand `{value}`; expected presets, run-suite, run, record, status, gate, summary, trends, baseline-template, compare, baselines, list, show, clean, or scorecard"
        ),
    }
}

fn benchmark_status_args_request_failure(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--fail-on-not-ready" | "--fail-on-gaps" | "--strict"
        )
    })
}

fn benchmark_args_are_scorecard_compatible(args: &[String]) -> bool {
    args.is_empty()
        || args.first().is_some_and(|arg| {
            matches!(
                arg.as_str(),
                "--json" | "--output" | "-o" | "--fail-below" | "--min-score"
            ) || arg.starts_with("--output=")
                || arg.starts_with("--fail-below=")
                || arg.starts_with("--min-score=")
        })
}
