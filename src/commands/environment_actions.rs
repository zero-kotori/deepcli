use super::with_smoke;
use crate::tools::{DiscoveredTestCommand, EnvironmentReport};

pub(crate) fn environment_next_actions(
    environment: Option<&EnvironmentReport>,
    tests: &[DiscoveredTestCommand],
) -> Vec<String> {
    let mut actions = Vec::new();
    let compiler_docker_test = tests.iter().any(|command| {
        command.requires_docker
            && command.command.contains("maxxing/compiler-dev")
            && command.command.contains("autotest")
    });

    match environment {
        Some(report) if report.ready => {
            if compiler_docker_test {
                actions.push("deepcli compiler test --json".to_string());
            }
        }
        Some(report) => {
            if let Some(action) = &report.recommended_action {
                let action = with_smoke(action);
                actions.push(shell_command_from_slash_command(&action));
            }
            if compiler_docker_test {
                actions.push("deepcli compiler test --json".to_string());
            }
        }
        None => actions.extend(default_environment_next_actions()),
    }
    if actions.is_empty() {
        actions.extend(default_environment_next_actions());
    }
    actions
}

fn default_environment_next_actions() -> Vec<String> {
    vec![
        "deepcli doctor docker --json".to_string(),
        "deepcli install docker --smoke".to_string(),
    ]
}

fn shell_command_from_slash_command(action: &str) -> String {
    action
        .strip_prefix('/')
        .map(|command| format!("deepcli {command}"))
        .unwrap_or_else(|| action.to_string())
}
