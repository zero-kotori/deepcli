use super::test_discovery::discover_tests_in;
use crate::privacy::redact_sensitive_text;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

const APPROVAL_SUMMARY_CHARS: usize = 160;

pub(super) fn invocation_digest(tool: &str, args: &Value) -> String {
    let mut canonical = String::new();
    write_canonical_json(args, &mut canonical);
    let mut hasher = Sha256::new();
    hasher.update(tool.as_bytes());
    hasher.update([0]);
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(super) fn invocation_summary(tool: &str, args: &Value) -> String {
    if tool == "git_commit" {
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let files = args
            .get("_staged_files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .take(3)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let file_count = args
            .get("_staged_files")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        let digest = args
            .get("_staged_digest")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return truncate_summary(&format!(
            "message={} staged_files={} [{}] tree={}",
            redact_sensitive_text(message),
            file_count,
            files.join(", "),
            digest.chars().take(12).collect::<String>()
        ));
    }
    const SUMMARY_KEYS: &[&str] = &[
        "command", "path", "name", "target", "url", "query", "task", "message",
    ];
    for key in SUMMARY_KEYS {
        if let Some(value) = args.get(*key).and_then(Value::as_str) {
            let value = redact_sensitive_text(value.trim());
            if *key == "command" {
                return command_summary(&value);
            }
            let mut summary = format!("{key}={value}");
            if *key == "path" {
                if let Some(bytes) = args.get("_content_bytes").and_then(Value::as_u64) {
                    summary.push_str(&format!(" bytes={bytes}"));
                }
                if let Some(lines) = args.get("_content_lines").and_then(Value::as_u64) {
                    summary.push_str(&format!(" lines={lines}"));
                }
            }
            return truncate_summary(&summary);
        }
    }
    if let Some(targets) = args.get("targets").and_then(Value::as_array) {
        let targets = targets
            .iter()
            .filter_map(Value::as_str)
            .take(4)
            .collect::<Vec<_>>()
            .join(", ");
        let patch = args
            .get("patch")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let added = patch.lines().filter(|line| line.starts_with('+')).count();
        let removed = patch.lines().filter(|line| line.starts_with('-')).count();
        return truncate_summary(&format!(
            "targets={targets} added_lines={added} removed_lines={removed}"
        ));
    }
    let digest = invocation_digest(tool, args);
    format!("invocation={}", &digest[..12])
}

fn command_summary(command: &str) -> String {
    let chars = command.chars().collect::<Vec<_>>();
    if chars.len() <= APPROVAL_SUMMARY_CHARS.saturating_sub(8) {
        return format!("command={command}");
    }
    let head = chars.iter().take(72).collect::<String>();
    let tail = chars
        .iter()
        .skip(chars.len().saturating_sub(64))
        .collect::<String>();
    format!("command={head}...{tail}")
}

pub(crate) fn contains_shell_control(command: &str) -> bool {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let chars = command.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\n' || ch == '\r' || ch == '\0' {
            return true;
        }
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single_quoted {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double_quoted {
            single_quoted = !single_quoted;
            index += 1;
            continue;
        }
        if ch == '"' && !single_quoted {
            double_quoted = !double_quoted;
            index += 1;
            continue;
        }
        if !single_quoted {
            if ch == '`' {
                return true;
            }
            if ch == '$' && chars.get(index + 1) == Some(&'(') {
                return true;
            }
        }
        if !single_quoted && !double_quoted && matches!(ch, ';' | '|' | '&' | '>' | '<') {
            return true;
        }
        index += 1;
    }
    single_quoted || double_quoted || escaped
}

pub(super) fn validate_test_command(workspace: &Path, command: &str) -> Result<()> {
    if command.trim().is_empty() {
        bail!("test command cannot be empty");
    }
    if contains_shell_control(command) {
        bail!("test command cannot contain shell control operators or substitutions");
    }
    let requested = shell_words::split(command).context("failed to parse test command")?;
    let discovered = discover_tests_in(workspace)?;
    let prefix_len = discovered.iter().find_map(|candidate| {
        shell_words::split(&candidate.command)
            .ok()
            .filter(|prefix| requested.starts_with(prefix))
            .map(|prefix| prefix.len())
    });
    let Some(prefix_len) = prefix_len else {
        bail!(
            "test command must extend a command discovered from the current workspace; run discover_tests first"
        );
    };
    validate_test_command_extension(&requested[..prefix_len], &requested[prefix_len..])?;
    Ok(())
}

fn validate_test_command_extension(prefix: &[String], extension: &[String]) -> Result<()> {
    if extension.is_empty() {
        return Ok(());
    }
    if prefix.first().map(String::as_str) != Some("cargo") {
        bail!("custom test arguments are only supported for discovered cargo test commands");
    }
    validate_cargo_test_extension(extension)
}

fn validate_cargo_test_extension(extension: &[String]) -> Result<()> {
    const CARGO_SWITCHES: &[&str] = &[
        "--all",
        "--all-features",
        "--all-targets",
        "--benches",
        "--bins",
        "--doc",
        "--examples",
        "--frozen",
        "--ignore-rust-version",
        "--lib",
        "--locked",
        "--no-default-features",
        "--no-fail-fast",
        "--no-run",
        "--offline",
        "--quiet",
        "--release",
        "--tests",
        "--verbose",
        "--workspace",
        "-q",
        "-v",
    ];
    const CARGO_VALUE_FLAGS: &[&str] = &[
        "--bench",
        "--bin",
        "--color",
        "--example",
        "--exclude",
        "--features",
        "--jobs",
        "--message-format",
        "--package",
        "--profile",
        "--test",
        "-j",
        "-p",
    ];
    const LIBTEST_SWITCHES: &[&str] = &[
        "--exact",
        "--ignored",
        "--include-ignored",
        "--list",
        "--nocapture",
        "--quiet",
        "--show-output",
    ];
    const LIBTEST_VALUE_FLAGS: &[&str] = &["--color", "--format", "--skip", "--test-threads"];

    let mut libtest = false;
    let mut index = 0usize;
    while index < extension.len() {
        let argument = &extension[index];
        if argument == "--" && !libtest {
            libtest = true;
            index += 1;
            continue;
        }
        let (switches, value_flags) = if libtest {
            (LIBTEST_SWITCHES, LIBTEST_VALUE_FLAGS)
        } else {
            (CARGO_SWITCHES, CARGO_VALUE_FLAGS)
        };
        if switches.contains(&argument.as_str()) {
            index += 1;
            continue;
        }
        if let Some((flag, value)) = argument.split_once('=') {
            if value_flags.contains(&flag) {
                validate_safe_test_argument(value)?;
                index += 1;
                continue;
            }
        }
        if value_flags.contains(&argument.as_str()) {
            let value = extension
                .get(index + 1)
                .ok_or_else(|| anyhow::anyhow!("test option {argument} requires a value"))?;
            validate_safe_test_argument(value)?;
            index += 2;
            continue;
        }
        if argument.starts_with('-') {
            bail!("unsupported test option: {argument}");
        }
        validate_safe_test_argument(argument)?;
        index += 1;
    }
    Ok(())
}

fn validate_safe_test_argument(argument: &str) -> Result<()> {
    if argument.is_empty() {
        bail!("test command arguments cannot be empty");
    }
    let path = Path::new(argument);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("test command argument must stay inside the current workspace: {argument}");
    }
    Ok(())
}

pub(crate) fn is_test_or_build_command(command: &str) -> bool {
    if contains_shell_control(command) {
        return false;
    }
    let Ok(parts) = shell_words::split(command) else {
        return false;
    };
    match parts.as_slice() {
        [program, action, ..]
            if program == "cargo"
                && matches!(action.as_str(), "test" | "check" | "clippy" | "build") =>
        {
            true
        }
        [program, action, ..]
            if matches!(program.as_str(), "npm" | "pnpm" | "yarn" | "bun") && action == "test" =>
        {
            true
        }
        [program, ..] if matches!(program.as_str(), "pytest" | "ctest") => true,
        [python, dash_m, module, ..]
            if matches!(python.as_str(), "python" | "python3")
                && dash_m == "-m"
                && module == "pytest" =>
        {
            true
        }
        [program, action, ..] if program == "go" && action == "test" => true,
        [program, action, ..] if program == "dotnet" && action == "test" => true,
        [program, action, ..]
            if matches!(program.as_str(), "make" | "gmake") && action == "test" =>
        {
            true
        }
        _ => false,
    }
}

pub(crate) fn shell_command_requires_network(command: &str) -> bool {
    let Ok(parts) = shell_words::split(command) else {
        return true;
    };
    let Some(program) = parts.first().map(String::as_str) else {
        return false;
    };
    if matches!(
        program,
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "rsync" | "nc" | "ncat" | "telnet" | "ftp"
    ) {
        return true;
    }
    if program == "git" {
        return parts.get(1).is_some_and(|action| {
            matches!(
                action.as_str(),
                "clone" | "fetch" | "pull" | "push" | "ls-remote" | "submodule"
            )
        });
    }
    if matches!(program, "npm" | "pnpm" | "yarn" | "bun" | "pip" | "pip3") {
        return parts.get(1).is_some_and(|action| {
            matches!(
                action.as_str(),
                "install" | "add" | "publish" | "login" | "whoami"
            )
        });
    }
    program == "docker"
        && parts
            .get(1)
            .is_some_and(|action| matches!(action.as_str(), "pull" | "push" | "login" | "run"))
}

fn write_canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output.push_str(
            &serde_json::to_string(value).expect("serializing a JSON string cannot fail"),
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("serializing a JSON key cannot fail"),
                );
                output.push(':');
                write_canonical_json(&values[key], output);
            }
            output.push('}');
        }
    }
}

fn truncate_summary(value: &str) -> String {
    let mut output = value
        .chars()
        .take(APPROVAL_SUMMARY_CHARS)
        .collect::<String>();
    if value.chars().count() > APPROVAL_SUMMARY_CHARS {
        output.push_str("...");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn invocation_digest_binds_tool_and_canonical_arguments() {
        let left = invocation_digest("write_file", &json!({"content": "x", "path": "a"}));
        let reordered = invocation_digest("write_file", &json!({"path": "a", "content": "x"}));
        let changed = invocation_digest("write_file", &json!({"path": "a", "content": "y"}));
        let other_tool = invocation_digest(
            "apply_patch_or_write",
            &json!({"path": "a", "content": "x"}),
        );

        assert_eq!(left, reordered);
        assert_ne!(left, changed);
        assert_ne!(left, other_tool);
        assert_eq!(left.len(), 64);
    }

    #[test]
    fn git_commit_summary_identifies_the_approved_tree_and_files() {
        let summary = invocation_summary(
            "git_commit",
            &json!({
                "message": "update runtime",
                "_staged_files": ["src/runtime.rs", "src/tools.rs"],
                "_staged_digest": "0123456789abcdef"
            }),
        );

        assert!(summary.contains("staged_files=2"));
        assert!(summary.contains("src/runtime.rs"));
        assert!(summary.contains("tree=0123456789ab"));
    }

    #[test]
    fn invocation_summary_is_bounded_and_redacted() {
        let summary = invocation_summary(
            "run_shell",
            &json!({"command": "printf ok; api_key = sk-secret-value"}),
        );

        assert!(summary.starts_with("command="));
        assert!(!summary.contains("sk-secret-value"));
        assert!(summary.chars().count() <= 180);
    }

    #[test]
    fn shell_control_detection_handles_quotes_and_substitutions() {
        assert!(!contains_shell_control("cargo test 'a;b'"));
        assert!(contains_shell_control("cargo test; rm -rf target"));
        assert!(contains_shell_control("cargo test && echo done"));
        assert!(contains_shell_control("cargo test $(cat /etc/passwd)"));
        assert!(contains_shell_control("cargo test `cat /etc/passwd`"));
        assert!(contains_shell_control("cargo test\nrm -rf target"));
    }

    #[test]
    fn test_command_must_extend_a_discovered_command_without_shell_control() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();

        assert!(validate_test_command(dir.path(), "cargo test parser").is_ok());
        assert!(validate_test_command(dir.path(), "cargo test -p parser --quiet").is_ok());
        assert!(validate_test_command(dir.path(), "cargo test -- --nocapture").is_ok());
        assert!(validate_test_command(dir.path(), "printf ok").is_err());
        assert!(validate_test_command(dir.path(), "cargo test; rm -rf target").is_err());
        assert!(validate_test_command(dir.path(), "cargo test && curl example.com").is_err());
    }

    #[test]
    fn test_command_cannot_redirect_execution_outside_the_discovered_project() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'\n").unwrap();
        fs::write(dir.path().join("Makefile"), "test:\n\t@true\n").unwrap();
        fs::write(dir.path().join("pyproject.toml"), "[project]\nname='x'\n").unwrap();

        for command in [
            "cargo test --manifest-path ../outside/Cargo.toml",
            "cargo test --manifest-path=/tmp/outside/Cargo.toml",
            "cargo test --config target.aarch64-apple-darwin.runner=evil",
            "cargo test /tmp/outside.rs",
            "cargo test --target-dir /tmp/target",
            "make test deploy",
            "python -m pytest --junitxml=/tmp/results.xml",
        ] {
            assert!(
                validate_test_command(dir.path(), command).is_err(),
                "{command}"
            );
        }
    }
}
