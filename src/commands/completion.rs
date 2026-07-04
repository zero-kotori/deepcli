use super::{
    command_group_policy_json, dedup_preserve_order, legacy_command_policy_json,
    local_action_checklist, required_arg, set_command_output_path, write_command_output,
    CommandGroup, CommandRouter,
};
use crate::schema_ids;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum CompletionFormat {
    #[default]
    Guide,
    Bash,
    Zsh,
    Fish,
    Json,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CompletionOptions {
    format: CompletionFormat,
    install: bool,
    status: bool,
    force: bool,
    dry_run: bool,
    json_output: bool,
    output_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct CompletionCommand {
    name: String,
    summary: String,
    running_safe: bool,
    group: CommandGroup,
}

#[derive(Debug, Clone)]
pub(super) struct CompletionInstallReport {
    pub(super) report: String,
    pub(super) shell: CompletionFormat,
    pub(super) target_path: PathBuf,
    pub(super) status: String,
    pub(super) dry_run: bool,
    pub(super) force: bool,
    pub(super) bytes: usize,
    pub(super) parent_created: bool,
    pub(super) next_actions: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct CompletionStatusReport {
    pub(super) report: String,
    pub(super) shell: CompletionFormat,
    pub(super) target_path: PathBuf,
    pub(super) status: String,
    pub(super) installed: bool,
    pub(super) up_to_date: bool,
    pub(super) expected_bytes: usize,
    pub(super) installed_bytes: Option<usize>,
    pub(super) next_actions: Vec<String>,
}

pub(super) fn handle_completion(workspace: &Path, args: Vec<String>) -> Result<String> {
    let options = parse_completion_options(&args)?;
    let commands = completion_commands();
    let output = if options.install {
        let shell = completion_install_shell(options.format)?;
        let script = format_completion_script(shell, &commands)?;
        let report = install_completion_script(shell, &script, options.force, options.dry_run)?;
        if options.json_output {
            format_completion_install_json(&report)?
        } else {
            report.report
        }
    } else if options.status {
        let shell = completion_status_shell(options.format)?;
        let script = format_completion_script(shell, &commands)?;
        let report = completion_status_report(shell, &script)?;
        if options.json_output {
            format_completion_status_json(&report)?
        } else {
            report.report
        }
    } else {
        match options.format {
            CompletionFormat::Guide => format_completion_guide(commands.len()),
            CompletionFormat::Bash | CompletionFormat::Zsh | CompletionFormat::Fish => {
                format_completion_script(options.format, &commands)?
            }
            CompletionFormat::Json => format_completion_json(&commands)?,
        }
    };
    if let Some(output_path) = &options.output_path {
        write_command_output(workspace, output_path, &output)?;
    }
    Ok(output)
}

pub(super) fn format_completion_script(
    format: CompletionFormat,
    commands: &[CompletionCommand],
) -> Result<String> {
    Ok(match format {
        CompletionFormat::Guide => format_completion_guide(commands.len()),
        CompletionFormat::Bash => format_bash_completion(commands),
        CompletionFormat::Zsh => format_zsh_completion(commands),
        CompletionFormat::Fish => format_fish_completion(commands),
        CompletionFormat::Json => bail!("json is a command catalog, not a shell script"),
    })
}

pub(crate) fn handle_completion_local(workspace: &Path, args: Vec<String>) -> Result<String> {
    handle_completion(workspace, args)
}

fn parse_completion_options(args: &[String]) -> Result<CompletionOptions> {
    let mut options = CompletionOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "install" => {
                if options.install || options.status {
                    bail!("multiple /completion actions were provided");
                }
                options.install = true;
                index += 1;
            }
            "status" | "check" => {
                if options.install || options.status {
                    bail!("multiple /completion actions were provided");
                }
                options.status = true;
                index += 1;
            }
            "bash" | "zsh" | "fish" | "json" => {
                if options.install && args[index] == "json" {
                    bail!("completion install shell must be bash, zsh, or fish; use --json for an install report");
                }
                if options.status && args[index] == "json" {
                    bail!("completion status shell must be bash, zsh, or fish; use --json for a status report");
                }
                set_completion_format(&mut options.format, parse_completion_format(&args[index])?)?;
                index += 1;
            }
            "--json" => {
                if options.install || options.status {
                    options.json_output = true;
                } else {
                    set_completion_format(&mut options.format, CompletionFormat::Json)?;
                }
                index += 1;
            }
            "--force" => {
                options.force = true;
                index += 1;
            }
            "--dry-run" => {
                options.dry_run = true;
                index += 1;
            }
            "--shell" | "--format" => {
                let raw = required_arg(args, index + 1, "shell")?;
                set_completion_format(&mut options.format, parse_completion_format(raw)?)?;
                index += 2;
            }
            value if value.starts_with("--shell=") => {
                set_completion_format(
                    &mut options.format,
                    parse_completion_format(value.trim_start_matches("--shell="))?,
                )?;
                index += 1;
            }
            value if value.starts_with("--format=") => {
                set_completion_format(
                    &mut options.format,
                    parse_completion_format(value.trim_start_matches("--format="))?,
                )?;
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
            value => bail!("unsupported /completion option `{value}`"),
        }
    }
    if options.install && matches!(options.format, CompletionFormat::Json) {
        bail!(
            "completion install shell must be bash, zsh, or fish; use --json for an install report"
        );
    }
    if options.status && matches!(options.format, CompletionFormat::Json) {
        bail!("completion status shell must be bash, zsh, or fish; use --json for a status report");
    }
    if options.force && options.dry_run {
        bail!("--force cannot be combined with --dry-run");
    }
    if options.status && (options.force || options.dry_run) {
        bail!("completion status does not accept --force or --dry-run");
    }
    Ok(options)
}

fn parse_completion_format(raw: &str) -> Result<CompletionFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "bash" => Ok(CompletionFormat::Bash),
        "zsh" => Ok(CompletionFormat::Zsh),
        "fish" => Ok(CompletionFormat::Fish),
        "json" => Ok(CompletionFormat::Json),
        value => bail!("unsupported completion format `{value}`"),
    }
}

fn set_completion_format(current: &mut CompletionFormat, next: CompletionFormat) -> Result<()> {
    if *current != CompletionFormat::Guide && *current != next {
        bail!("conflicting /completion formats were provided");
    }
    *current = next;
    Ok(())
}

fn completion_install_shell(format: CompletionFormat) -> Result<CompletionFormat> {
    match format {
        CompletionFormat::Guide => Ok(detect_completion_shell()),
        CompletionFormat::Bash | CompletionFormat::Zsh | CompletionFormat::Fish => Ok(format),
        CompletionFormat::Json => {
            bail!("completion install shell must be bash, zsh, or fish; use --json for an install report")
        }
    }
}

fn completion_status_shell(format: CompletionFormat) -> Result<CompletionFormat> {
    match format {
        CompletionFormat::Guide => Ok(detect_completion_shell()),
        CompletionFormat::Bash | CompletionFormat::Zsh | CompletionFormat::Fish => Ok(format),
        CompletionFormat::Json => {
            bail!("completion status shell must be bash, zsh, or fish; use --json for a status report")
        }
    }
}

fn detect_completion_shell() -> CompletionFormat {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("fish") {
        CompletionFormat::Fish
    } else if shell.ends_with("bash") {
        CompletionFormat::Bash
    } else {
        CompletionFormat::Zsh
    }
}

pub(super) fn completion_commands() -> Vec<CompletionCommand> {
    let mut commands = Vec::new();
    for summary in CommandRouter::help_summaries() {
        let name = summary.name.trim_start_matches('/').to_string();
        add_completion_command(
            &mut commands,
            name,
            summary.summary.to_string(),
            summary.running_safe,
            summary.group,
        );
    }

    for alias in CommandRouter::completion_alias_metadata() {
        add_completion_command(
            &mut commands,
            alias.name.to_string(),
            alias.summary.to_string(),
            alias.running_safe,
            alias.group,
        );
    }
    commands
}

fn add_completion_command(
    commands: &mut Vec<CompletionCommand>,
    name: String,
    summary: String,
    running_safe: bool,
    group: CommandGroup,
) {
    if commands.iter().any(|command| command.name == name) {
        return;
    }
    commands.push(CompletionCommand {
        name,
        summary,
        running_safe,
        group,
    });
}

fn format_completion_guide(command_count: usize) -> String {
    [
        "deepcli completion".to_string(),
        format!("commands: {command_count}"),
        "one-step install:".to_string(),
        "  deepcli completion status zsh".to_string(),
        "  deepcli completion install zsh --force".to_string(),
        "  deepcli completion install bash --force".to_string(),
        "  deepcli completion install fish --force".to_string(),
        "install examples:".to_string(),
        "  deepcli completion zsh > ~/.zsh/completions/_deepcli".to_string(),
        "  deepcli completion bash > ~/.local/share/bash-completion/completions/deepcli"
            .to_string(),
        "  deepcli completion fish > ~/.config/fish/completions/deepcli.fish".to_string(),
        "machine-readable catalog:".to_string(),
        "  deepcli completion json --output .deepcli/exports/commands.json".to_string(),
        "notes:".to_string(),
        "  - no session is created and no provider is called".to_string(),
        "  - use /completion [bash|zsh|fish|json] inside native terminal chat".to_string(),
    ]
    .join("\n")
}

fn install_completion_script(
    shell: CompletionFormat,
    script: &str,
    force: bool,
    explicit_dry_run: bool,
) -> Result<CompletionInstallReport> {
    let home =
        dirs::home_dir().context("failed to determine home directory for completion install")?;
    install_completion_script_in(&home, shell, script, force, explicit_dry_run)
}

pub(super) fn install_completion_script_in(
    home: &Path,
    shell: CompletionFormat,
    script: &str,
    force: bool,
    explicit_dry_run: bool,
) -> Result<CompletionInstallReport> {
    let target_path = completion_install_target(home, shell)?;
    let dry_run = explicit_dry_run || !force;
    let parent_existed = target_path.parent().is_some_and(Path::exists);
    let bytes = script.len();
    let mut status = "dry_run".to_string();
    if !dry_run {
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if target_path.exists() && fs::read_to_string(&target_path).unwrap_or_default() == script {
            status = "up_to_date".to_string();
        } else {
            fs::write(&target_path, script)
                .with_context(|| format!("failed to write {}", target_path.display()))?;
            status = "installed".to_string();
        }
    }
    let parent_created =
        !dry_run && !parent_existed && target_path.parent().is_some_and(Path::exists);
    let next_actions = completion_install_next_actions(shell, dry_run);
    let mut lines = vec![
        "deepcli completion install".to_string(),
        format!("shell: {}", completion_shell_name(shell)),
        format!("target: {}", target_path.display()),
        format!("status: {status}"),
        format!("bytes: {bytes}"),
    ];
    if dry_run {
        lines.push("write: skipped (dry-run; add --force to install)".to_string());
    } else {
        lines.push("write: done".to_string());
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));

    Ok(CompletionInstallReport {
        report: lines.join("\n"),
        shell,
        target_path,
        status,
        dry_run,
        force,
        bytes,
        parent_created,
        next_actions,
    })
}

fn completion_status_report(
    shell: CompletionFormat,
    expected_script: &str,
) -> Result<CompletionStatusReport> {
    let home =
        dirs::home_dir().context("failed to determine home directory for completion status")?;
    completion_status_report_in(&home, shell, expected_script)
}

pub(super) fn completion_status_report_in(
    home: &Path,
    shell: CompletionFormat,
    expected_script: &str,
) -> Result<CompletionStatusReport> {
    let target_path = completion_install_target(home, shell)?;
    let expected_bytes = expected_script.len();
    let current = fs::read_to_string(&target_path).ok();
    let installed_bytes = current.as_ref().map(|content| content.len());
    let installed = current.is_some();
    let up_to_date = current.as_deref() == Some(expected_script);
    let status = if up_to_date {
        "up_to_date"
    } else if installed {
        "stale"
    } else {
        "missing"
    }
    .to_string();
    let next_actions = completion_status_next_actions(shell, &status);
    let mut lines = vec![
        "deepcli completion status".to_string(),
        format!("shell: {}", completion_shell_name(shell)),
        format!("target: {}", target_path.display()),
        format!("status: {status}"),
        format!("expected bytes: {expected_bytes}"),
    ];
    if let Some(bytes) = installed_bytes {
        lines.push(format!("installed bytes: {bytes}"));
    } else {
        lines.push("installed bytes: <none>".to_string());
    }
    lines.push("next actions:".to_string());
    lines.extend(next_actions.iter().map(|action| format!("  - {action}")));

    Ok(CompletionStatusReport {
        report: lines.join("\n"),
        shell,
        target_path,
        status,
        installed,
        up_to_date,
        expected_bytes,
        installed_bytes,
        next_actions,
    })
}

fn completion_status_next_actions(shell: CompletionFormat, status: &str) -> Vec<String> {
    let shell_name = completion_shell_name(shell);
    match status {
        "up_to_date" => vec![
            "deepcli doctor shell --json".to_string(),
            format!("deepcli completion status {shell_name} --json"),
        ],
        "stale" | "missing" => vec![
            format!("deepcli completion install {shell_name} --force"),
            format!("deepcli completion status {shell_name} --json"),
        ],
        _ => vec![format!("deepcli completion status {shell_name} --json")],
    }
}

pub(super) fn completion_install_target(home: &Path, shell: CompletionFormat) -> Result<PathBuf> {
    Ok(match shell {
        CompletionFormat::Zsh => home.join(".zsh").join("completions").join("_deepcli"),
        CompletionFormat::Bash => home
            .join(".local")
            .join("share")
            .join("bash-completion")
            .join("completions")
            .join("deepcli"),
        CompletionFormat::Fish => home
            .join(".config")
            .join("fish")
            .join("completions")
            .join("deepcli.fish"),
        CompletionFormat::Guide | CompletionFormat::Json => {
            bail!("completion install target requires bash, zsh, or fish")
        }
    })
}

fn completion_install_next_actions(shell: CompletionFormat, dry_run: bool) -> Vec<String> {
    let shell_name = completion_shell_name(shell);
    let mut actions = Vec::new();
    if dry_run {
        actions.push(format!("deepcli completion install {shell_name} --force"));
    }
    actions.push(format!("deepcli completion status {shell_name} --json"));
    actions.push("deepcli doctor shell --json".to_string());
    dedup_preserve_order(actions)
}

pub(super) fn format_completion_install_json(report: &CompletionInstallReport) -> Result<String> {
    let checklist = local_action_checklist(&report.next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::COMPLETION_INSTALL_V1,
        "program": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "shell": completion_shell_name(report.shell),
        "targetPath": report.target_path.display().to_string(),
        "status": report.status,
        "dryRun": report.dry_run,
        "force": report.force,
        "bytes": report.bytes,
        "parentCreated": report.parent_created,
        "nextActions": &report.next_actions,
        "checklist": checklist,
        "report": report.report,
    }))?)
}

pub(super) fn format_completion_status_json(report: &CompletionStatusReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(
        &completion_status_json_value(report),
    )?)
}

pub(super) fn completion_status_json_value(report: &CompletionStatusReport) -> Value {
    let checklist = local_action_checklist(&report.next_actions);
    json!({
        "schema": schema_ids::COMPLETION_STATUS_V1,
        "program": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "shell": completion_shell_name(report.shell),
        "targetPath": report.target_path.display().to_string(),
        "status": report.status,
        "installed": report.installed,
        "upToDate": report.up_to_date,
        "expectedBytes": report.expected_bytes,
        "installedBytes": report.installed_bytes,
        "nextActions": &report.next_actions,
        "checklist": checklist,
        "report": report.report,
    })
}

pub(super) fn completion_shell_name(shell: CompletionFormat) -> &'static str {
    match shell {
        CompletionFormat::Bash => "bash",
        CompletionFormat::Zsh => "zsh",
        CompletionFormat::Fish => "fish",
        CompletionFormat::Json => "json",
        CompletionFormat::Guide => "guide",
    }
}

fn format_bash_completion(commands: &[CompletionCommand]) -> String {
    let command_words = completion_words(commands);
    let provider_command_words = provider_completion_words();
    [
        "# deepcli bash completion".to_string(),
        "_deepcli() {".to_string(),
        "  local cur command".to_string(),
        "  COMPREPLY=()".to_string(),
        "  cur=\"${COMP_WORDS[COMP_CWORD]}\"".to_string(),
        "  command=\"${COMP_WORDS[1]}\"".to_string(),
        "  if [[ ${COMP_CWORD} -eq 1 ]]; then".to_string(),
        format!("    COMPREPLY=( $(compgen -W \"{command_words}\" -- \"$cur\") )"),
        "    return 0".to_string(),
        "  fi".to_string(),
        "  case \"$command\" in".to_string(),
        "    deepseek|kimi)".to_string(),
        "      if [[ ${COMP_CWORD} -eq 2 ]]; then".to_string(),
        format!(
            "        COMPREPLY=( $(compgen -W \"{provider_command_words}\" -- \"$cur\") )"
        ),
        "        return 0".to_string(),
        "      fi".to_string(),
        "      ;;".to_string(),
        "    model)".to_string(),
        "      COMPREPLY=( $(compgen -W \"deepseek kimi\" -- \"$cur\") )".to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "    doctor|health|diagnose|check|docker|compiler|setup|install|env)".to_string(),
        "      COMPREPLY=( $(compgen -W \"docker compiler check plan setup install test --json --output --quick --full-env --probe-provider\" -- \"$cur\") )".to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "    completion|completions)".to_string(),
        "      COMPREPLY=( $(compgen -W \"install status check bash zsh fish json --force --dry-run --json --output\" -- \"$cur\") )"
            .to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "    *)".to_string(),
        "      COMPREPLY=( $(compgen -W \"--json --output --limit --current --all --help\" -- \"$cur\") )".to_string(),
        "      return 0".to_string(),
        "      ;;".to_string(),
        "  esac".to_string(),
        "}".to_string(),
        "complete -F _deepcli deepcli".to_string(),
    ]
    .join("\n")
}

fn format_zsh_completion(commands: &[CompletionCommand]) -> String {
    let command_words = completion_words(commands);
    let provider_command_words = provider_completion_words();
    [
        "#compdef deepcli".to_string(),
        "# deepcli zsh completion".to_string(),
        "_deepcli() {".to_string(),
        "  local -a commands provider_commands providers env_words completion_words common_options".to_string(),
        format!("  commands=({command_words})"),
        format!("  provider_commands=({provider_command_words})"),
        "  providers=(deepseek kimi)".to_string(),
        "  env_words=(docker compiler check plan setup install test --json --output --quick --full-env --probe-provider)".to_string(),
        "  completion_words=(install status check bash zsh fish json --force --dry-run --json --output)"
            .to_string(),
        "  common_options=(--json --output --limit --current --all --help)".to_string(),
        "  if (( CURRENT == 2 )); then".to_string(),
        "    compadd -- ${commands[@]}".to_string(),
        "  elif [[ ${words[2]} == (deepseek|kimi) && CURRENT == 3 ]]; then".to_string(),
        "    compadd -- ${provider_commands[@]}".to_string(),
        "  elif [[ ${words[2]} == (model) ]]; then".to_string(),
        "    compadd -- ${providers[@]}".to_string(),
        "  elif [[ ${words[2]} == (doctor|health|diagnose|check|docker|compiler|setup|install|env) ]]; then".to_string(),
        "    compadd -- ${env_words[@]}".to_string(),
        "  elif [[ ${words[2]} == (completion|completions) ]]; then".to_string(),
        "    compadd -- ${completion_words[@]}".to_string(),
        "  else".to_string(),
        "    compadd -- ${common_options[@]}".to_string(),
        "  fi".to_string(),
        "}".to_string(),
        "_deepcli \"$@\"".to_string(),
    ]
    .join("\n")
}

fn format_fish_completion(commands: &[CompletionCommand]) -> String {
    let mut lines = vec![
        "# deepcli fish completion".to_string(),
        "complete -c deepcli -f".to_string(),
    ];
    for command in commands {
        lines.push(format!(
            "complete -c deepcli -n '__fish_use_subcommand' -a '{}' -d \"{}\"",
            command.name,
            fish_escape(&command.summary)
        ));
    }
    for provider in ["deepseek", "kimi"] {
        lines.push(format!(
            "complete -c deepcli -n '__fish_seen_subcommand_from {provider}' -a '{}' -d 'Provider command'",
            provider_completion_words()
        ));
    }
    lines.push("complete -c deepcli -n '__fish_seen_subcommand_from model provider use switch' -a 'deepseek kimi' -d 'Provider'".to_string());
    lines.push("complete -c deepcli -n '__fish_seen_subcommand_from doctor health diagnose check docker compiler setup install env' -a 'docker compiler check plan setup install test --json --output --quick --full-env --probe-provider' -d 'Environment or diagnostic argument'".to_string());
    lines.push("complete -c deepcli -n '__fish_seen_subcommand_from completion completions' -a 'install status check bash zsh fish json --force --dry-run --json --output' -d 'Completion format, status, or install option'".to_string());
    lines.push("complete -c deepcli -l json -d 'Output JSON where supported'".to_string());
    lines.push(
        "complete -c deepcli -l output -r -d 'Write output to a workspace-contained file'"
            .to_string(),
    );
    lines.join("\n")
}

fn format_completion_json(commands: &[CompletionCommand]) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::COMPLETION_V1,
        "program": "deepcli",
        "version": env!("CARGO_PKG_VERSION"),
        "shells": ["bash", "zsh", "fish"],
        "providers": ["deepseek", "kimi"],
        "groups": command_group_policy_json(),
        "legacyCommands": legacy_command_policy_json(),
        "install": [
            "deepcli completion status zsh",
            "deepcli completion status bash",
            "deepcli completion status fish",
            "deepcli completion install zsh --force",
            "deepcli completion install bash --force",
            "deepcli completion install fish --force",
            "deepcli completion zsh > ~/.zsh/completions/_deepcli",
            "deepcli completion bash > ~/.local/share/bash-completion/completions/deepcli",
            "deepcli completion fish > ~/.config/fish/completions/deepcli.fish"
        ],
        "commands": commands
            .iter()
            .map(|command| json!({
                "name": command.name,
                "summary": command.summary,
                "runningSafe": command.running_safe,
                "group": command.group.as_str(),
            }))
            .collect::<Vec<_>>(),
    }))?)
}

fn completion_words(commands: &[CompletionCommand]) -> String {
    commands
        .iter()
        .map(|command| command.name.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider_completion_words() -> &'static str {
    "ask stream resume repl version about quickstart recipes recipe playbook workflow workflows scorecard benchmark bench sota round iterate iteration selftest preflight release-check completion diagnose support health timeout model provider use switch models providers history cleanup accept gate login logout check docker compiler setup logs privacy"
}

fn fish_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
