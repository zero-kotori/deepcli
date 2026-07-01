use crate::config::GitIdentityConfig;
use crate::privacy::redact_sensitive_text;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command as ProcessCommand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitIdentityReport {
    pub(crate) git_present: bool,
    pub(crate) status: String,
    pub(crate) expected_name: Option<String>,
    pub(crate) expected_email: Option<String>,
    pub(crate) actual_name: Option<String>,
    pub(crate) actual_email: Option<String>,
    pub(crate) local_name: Option<String>,
    pub(crate) local_email: Option<String>,
    pub(crate) issues: Vec<String>,
    pub(crate) next_actions: Vec<String>,
}

pub(crate) fn build_git_identity_report(
    workspace: &Path,
    expected: &GitIdentityConfig,
) -> GitIdentityReport {
    let git_present = git_stdout(workspace, &["rev-parse", "--is-inside-work-tree"])
        .ok()
        .flatten()
        .as_deref()
        .is_some_and(|value| value.trim() == "true");
    let expected_name = normalize_optional_config_value(expected.user_name.as_deref());
    let expected_email = normalize_optional_config_value(expected.user_email.as_deref());
    let (actual_name, actual_email, local_name, local_email) = if git_present {
        (
            git_config_value(workspace, &["config", "--get", "user.name"]),
            git_config_value(workspace, &["config", "--get", "user.email"]),
            git_config_value(workspace, &["config", "--local", "--get", "user.name"]),
            git_config_value(workspace, &["config", "--local", "--get", "user.email"]),
        )
    } else {
        (None, None, None, None)
    };
    let expected_configured = expected_name.is_some() || expected_email.is_some();

    let mut issues = Vec::new();
    if git_present && expected_configured {
        if let Some(expected) = &expected_name {
            match actual_name.as_deref() {
                Some(actual) if actual == expected => {}
                Some(actual) => issues.push(format!(
                    "git user.name is `{}`; expected `{}`",
                    redact_sensitive_text(actual),
                    redact_sensitive_text(expected)
                )),
                None => issues.push(format!(
                    "git user.name is missing; expected `{}`",
                    redact_sensitive_text(expected)
                )),
            }
        }
        if let Some(expected) = &expected_email {
            match actual_email.as_deref() {
                Some(actual) if actual == expected => {}
                Some(actual) => issues.push(format!(
                    "git user.email is `{}`; expected `{}`",
                    redact_sensitive_text(actual),
                    redact_sensitive_text(expected)
                )),
                None => issues.push(format!(
                    "git user.email is missing; expected `{}`",
                    redact_sensitive_text(expected)
                )),
            }
        }
    }

    let status = if !git_present {
        "no_git"
    } else if !expected_configured {
        "unconfigured"
    } else if issues.is_empty() {
        "ok"
    } else {
        "mismatch"
    }
    .to_string();

    let mut next_actions = Vec::new();
    if git_present && expected_configured && !issues.is_empty() {
        next_actions.push("fix repo git identity before committing".to_string());
        if let Some(expected) = &expected_name {
            next_actions.push(format!(
                "run `git config user.name {}` in this repo",
                shell_words::quote(expected)
            ));
        }
        if let Some(expected) = &expected_email {
            next_actions.push(format!(
                "run `git config user.email {}` in this repo",
                shell_words::quote(expected)
            ));
        }
    } else if git_present && expected_configured && status == "ok" {
        if expected_name.as_deref() != local_name.as_deref()
            || expected_email.as_deref() != local_email.as_deref()
        {
            next_actions.push(
                "optionally pin matching git identity in this repo with `git config user.name ...` and `git config user.email ...`"
                    .to_string(),
            );
        }
    } else if git_present {
        next_actions.push(
            "configure `project.gitIdentity` in `.deepcli/config.json` to make doctor/selftest guard commit identity".to_string(),
        );
    }
    GitIdentityReport {
        git_present,
        status,
        expected_name,
        expected_email,
        actual_name,
        actual_email,
        local_name,
        local_email,
        issues,
        next_actions,
    }
}

pub(crate) fn format_git_identity_summary(identity: &GitIdentityReport) -> String {
    if !identity.git_present {
        return format!("not a git repository status={}", identity.status);
    }
    let actual_name = identity.actual_name.as_deref().unwrap_or("<unset>");
    let actual_email = identity.actual_email.as_deref().unwrap_or("<unset>");
    let expected = match (
        identity.expected_name.as_deref(),
        identity.expected_email.as_deref(),
    ) {
        (Some(name), Some(email)) => format!(" expected={name} <{email}>"),
        (Some(name), None) => format!(" expected_name={name}"),
        (None, Some(email)) => format!(" expected_email={email}"),
        (None, None) => String::new(),
    };
    format!(
        "{} <{}> status={}{}",
        redact_sensitive_text(actual_name),
        redact_sensitive_text(actual_email),
        identity.status,
        expected
    )
}

pub(crate) fn git_identity_json(identity: &GitIdentityReport) -> Value {
    json!({
        "gitPresent": identity.git_present,
        "status": identity.status.as_str(),
        "expected": {
            "userName": identity.expected_name.as_deref(),
            "userEmail": identity.expected_email.as_deref(),
        },
        "actual": {
            "userName": identity.actual_name.as_deref(),
            "userEmail": identity.actual_email.as_deref(),
        },
        "local": {
            "userName": identity.local_name.as_deref(),
            "userEmail": identity.local_email.as_deref(),
        },
        "issues": &identity.issues,
        "nextActions": &identity.next_actions,
    })
}

pub(crate) fn git_stdout(workspace: &Path, args: &[&str]) -> Result<Option<String>> {
    Ok(git_stdout_bytes(workspace, args)?.map(|bytes| String::from_utf8_lossy(&bytes).to_string()))
}

pub(crate) fn git_stdout_bytes(workspace: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    let output = ProcessCommand::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(output.stdout))
}

fn normalize_optional_config_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn git_config_value(workspace: &Path, args: &[&str]) -> Option<String> {
    git_stdout(workspace, args)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
