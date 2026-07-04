use crate::config::{absolutize_workspace_path, AppConfig};

use super::monitor::{append_monitor_quick_actions, MonitorQuickAction};
use super::{compact_ui_text, header_status_for_state, workspace_for_state, TuiState};
use std::env;
use std::path::Path;

pub(super) fn health_quick_actions_for_state(state: &TuiState) -> Vec<MonitorQuickAction> {
    let Some(workspace) = workspace_for_state(state) else {
        return vec![
            MonitorQuickAction::run("/doctor --quick"),
            MonitorQuickAction::run("/config validate --json"),
        ];
    };
    let Ok(config) = AppConfig::load_effective(workspace, None) else {
        return vec![
            MonitorQuickAction::run("/config validate --json"),
            MonitorQuickAction::run("/doctor --quick"),
        ];
    };
    let header = header_status_for_state(state);
    let provider_name = if header.provider.starts_with('<') {
        config.default_provider.clone()
    } else {
        header.provider
    };
    let mut actions = vec![MonitorQuickAction::run("/model show --json")];
    if provider_needs_credentials_for_ui(workspace, &config, &provider_name) {
        actions.push(MonitorQuickAction::run(format!(
            "/credentials set {provider_name}"
        )));
    }
    actions.extend([
        MonitorQuickAction::run(format!("/credentials status {provider_name} --json")),
        MonitorQuickAction::run("/config validate --json"),
        MonitorQuickAction::run("/selftest --json"),
        MonitorQuickAction::run("/doctor --quick"),
    ]);
    actions
}

fn provider_needs_credentials_for_ui(
    workspace: &Path,
    config: &AppConfig,
    provider_name: &str,
) -> bool {
    if config.provider(Some(provider_name)).is_err() {
        return false;
    }
    match config.provider_runtime(workspace, Some(provider_name)) {
        Ok(runtime) => runtime.api_key.is_none(),
        Err(_) => true,
    }
}

pub(super) fn format_health_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let Some(workspace) = workspace_for_state(state) else {
        let mut lines = vec!["health unavailable: no workspace".to_string()];
        append_monitor_quick_actions(
            &mut lines,
            "quick actions",
            quick_actions,
            selected_quick_action,
        );
        return lines;
    };
    let header = header_status_for_state(state);
    let config = match AppConfig::load_effective(workspace, None) {
        Ok(config) => config,
        Err(error) => {
            let mut lines = vec![format!(
                "config load failed: {}",
                compact_ui_text(&error.to_string(), 100)
            )];
            append_monitor_quick_actions(
                &mut lines,
                "quick actions",
                quick_actions,
                selected_quick_action,
            );
            return lines;
        }
    };
    let provider_name = if header.provider.starts_with('<') {
        config.default_provider.clone()
    } else {
        header.provider.clone()
    };
    let mut lines = vec![format!(
        "provider: active={} model={} default={}",
        provider_name,
        header.model,
        compact_ui_text(&config.default_provider, 40)
    )];
    match config.provider(Some(&provider_name)) {
        Ok((name, provider)) => {
            let credentials_path = absolutize_workspace_path(workspace, &provider.credentials_file);
            let env_key = provider_env_key_for_ui(name);
            let env_present = env::var_os(&env_key).is_some();
            match config.provider_runtime(workspace, Some(name)) {
                Ok(runtime) => {
                    let api_key = if runtime.api_key.is_some() {
                        "configured"
                    } else {
                        "missing"
                    };
                    let model = runtime
                        .model
                        .as_deref()
                        .or(provider.acceptance_model.as_deref())
                        .unwrap_or("<unset>");
                    lines.push(format!(
                        "credentials: api_key={} file={} env={}",
                        api_key,
                        presence_label(credentials_path.exists()),
                        presence_label(env_present)
                    ));
                    lines.push(format!(
                        "runtime: type={} model={} endpoint={}",
                        runtime.provider_type,
                        compact_ui_text(model, 40),
                        runtime.endpoint.as_deref().unwrap_or("<default>")
                    ));
                }
                Err(error) => {
                    lines.push(format!(
                        "credentials: file={} env={} error={}",
                        presence_label(credentials_path.exists()),
                        presence_label(env_present),
                        compact_ui_text(&error.to_string(), 70)
                    ));
                }
            }
        }
        Err(error) => lines.push(format!(
            "provider config error: {}",
            compact_ui_text(&error.to_string(), 90)
        )),
    }
    lines.push(format!(
        "config: project={} permissions={} timeout={}s",
        presence_label(workspace.join(".deepcli/config.json").exists()),
        config.permissions.default_mode,
        config.agent.provider_turn_timeout_seconds
    ));
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

fn provider_env_key_for_ui(provider: &str) -> String {
    format!(
        "{}_API_KEY",
        provider.to_ascii_uppercase().replace('-', "_")
    )
}

fn presence_label(present: bool) -> &'static str {
    if present {
        "present"
    } else {
        "missing"
    }
}
