use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn validate_tool_arguments(name: &str, args: &Value) -> Result<()> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow!("tool `{name}` arguments must be a JSON object"))?;
    let allowed = allowed_arguments(name);
    if allowed.is_empty()
        && !matches!(
            name,
            "git_status"
                | "git_branch"
                | "discover_tests"
                | "prompt_list"
                | "skill_list"
                | "open_terminal"
        )
    {
        return Ok(());
    }
    for key in object.keys() {
        if !allowed.contains(key.as_str()) {
            bail!("unsupported argument `{key}` for tool `{name}`");
        }
    }
    for key in required_arguments(name) {
        if missing_value(object.get(*key)) {
            bail!("missing required argument `{key}` for tool `{name}`");
        }
    }
    validate_argument_types(name, args)?;
    Ok(())
}

fn allowed_arguments(name: &str) -> BTreeSet<&'static str> {
    match name {
        "read_file" => ["path", "start_line", "limit"].into(),
        "list_files" => ["path", "glob", "limit"].into(),
        "search" => [
            "query",
            "path",
            "glob",
            "limit",
            "case_sensitive",
            "context_lines",
            "max_file_bytes",
        ]
        .into(),
        "write_file" => ["path", "content", "approved"].into(),
        "apply_patch_or_write" => ["path", "content", "patch", "old", "new", "approved"].into(),
        "run_shell" => [
            "command",
            "approved",
            "writes_files",
            "requires_network",
            "timeout_seconds",
        ]
        .into(),
        "git_status" | "git_branch" | "discover_tests" | "prompt_list" | "skill_list"
        | "open_terminal" => BTreeSet::new(),
        "git_diff" => ["staged"].into(),
        "git_create_branch" => ["name", "approved"].into(),
        "git_commit_message" => BTreeSet::new(),
        "git_commit" => ["message", "approved"].into(),
        "run_tests" => ["command"].into(),
        "check_environment" => ["target"].into(),
        "setup_environment" => ["target", "approved", "install_missing", "smoke_test"].into(),
        "todo_write" => ["title", "todos"].into(),
        "ask_user_question" => ["question", "context"].into(),
        "web_search" => ["query"].into(),
        "web_fetch" => ["url", "max_chars"].into(),
        "prompt_get" => ["name"].into(),
        "prompt_render" => [
            "name",
            "file",
            "variables",
            "max_diff_chars",
            "max_file_chars",
        ]
        .into(),
        "skill_generate" => ["name", "description", "approved"].into(),
        "skill_run" => ["name"].into(),
        "spawn_subagent" => [
            "task",
            "depth",
            "write_scope",
            "read_scope",
            "allowed_tools",
            "context",
        ]
        .into(),
        _ => BTreeSet::new(),
    }
}

fn required_arguments(name: &str) -> &'static [&'static str] {
    match name {
        "read_file" => &["path"],
        "search" => &["query"],
        "write_file" => &["path", "content"],
        "run_shell" => &["command"],
        "git_create_branch" => &["name"],
        "git_commit" => &["message"],
        "todo_write" => &["todos"],
        "ask_user_question" => &["question"],
        "web_search" => &["query"],
        "web_fetch" => &["url"],
        "prompt_get" => &["name"],
        "prompt_render" => &["name"],
        "skill_generate" => &["name", "description"],
        "skill_run" => &["name"],
        "spawn_subagent" => &["task"],
        _ => &[],
    }
}

fn missing_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}

fn validate_argument_types(name: &str, args: &Value) -> Result<()> {
    match name {
        "read_file" => {
            expect_string(args, "path")?;
            expect_optional_positive_integer(args, "start_line")?;
            expect_optional_positive_integer(args, "limit")?;
        }
        "list_files" => {
            expect_optional_string(args, "path")?;
            expect_optional_string(args, "glob")?;
            expect_optional_positive_integer(args, "limit")?;
        }
        "search" => {
            expect_string(args, "query")?;
            expect_optional_string(args, "path")?;
            expect_optional_string(args, "glob")?;
            expect_optional_positive_integer(args, "limit")?;
            expect_optional_bool(args, "case_sensitive")?;
            expect_optional_integer(args, "context_lines")?;
            expect_optional_positive_integer(args, "max_file_bytes")?;
        }
        "todo_write" => validate_todos(args)?,
        "ask_user_question" => {
            expect_string(args, "question")?;
            expect_optional_string(args, "context")?;
        }
        "web_fetch" => {
            expect_string(args, "url")?;
            expect_optional_positive_integer(args, "max_chars")?;
        }
        "apply_patch_or_write" => validate_edit_mode(args)?,
        "spawn_subagent" => {
            expect_string(args, "task")?;
            expect_optional_positive_integer(args, "depth")?;
            expect_optional_string_array(args, "write_scope")?;
            expect_optional_string_array(args, "read_scope")?;
            expect_optional_string_array(args, "allowed_tools")?;
            expect_optional_string(args, "context")?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_edit_mode(args: &Value) -> Result<()> {
    let has_patch = args.get("patch").is_some();
    let has_content = args.get("content").is_some();
    let has_replace = args.get("old").is_some() || args.get("new").is_some();
    if has_patch {
        expect_string(args, "patch")?;
        expect_optional_string(args, "path")?;
        return Ok(());
    }
    if has_replace {
        expect_string(args, "path")?;
        expect_string(args, "old")?;
        expect_string(args, "new")?;
        return Ok(());
    }
    if has_content {
        expect_string(args, "path")?;
        expect_string(args, "content")?;
        return Ok(());
    }
    bail!("tool `apply_patch_or_write` requires `patch`, `content`, or `old`/`new` arguments")
}

fn validate_todos(args: &Value) -> Result<()> {
    let todos = args
        .get("todos")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("argument `todos` must be an array"))?;
    let mut ids = BTreeSet::new();
    for (index, item) in todos.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| anyhow!("todo item {} must be an object", index + 1))?;
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("todo item {} requires non-empty `content`", index + 1))?;
        let _ = content;
        if let Some(id) = object.get("id").and_then(Value::as_str) {
            if id.trim().is_empty() {
                bail!("todo item {} has an empty `id`", index + 1);
            }
            if !ids.insert(id.to_string()) {
                bail!("duplicate todo id `{id}`");
            }
        }
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        if !matches!(status, "pending" | "in_progress" | "completed" | "failed") {
            bail!("unsupported todo status `{status}`");
        }
    }
    Ok(())
}

fn expect_string(args: &Value, key: &str) -> Result<()> {
    if args.get(key).and_then(Value::as_str).is_some() {
        Ok(())
    } else {
        bail!("argument `{key}` must be a string")
    }
}

fn expect_optional_string(args: &Value, key: &str) -> Result<()> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(_)) => Ok(()),
        _ => bail!("argument `{key}` must be a string"),
    }
}

fn expect_optional_bool(args: &Value, key: &str) -> Result<()> {
    match args.get(key) {
        None | Some(Value::Null) | Some(Value::Bool(_)) => Ok(()),
        _ => bail!("argument `{key}` must be a boolean"),
    }
}

fn expect_optional_integer(args: &Value, key: &str) -> Result<()> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Number(value)) if value.is_i64() || value.is_u64() => Ok(()),
        _ => bail!("argument `{key}` must be an integer"),
    }
}

fn expect_optional_positive_integer(args: &Value, key: &str) -> Result<()> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Number(value)) if value.as_u64().is_some_and(|value| value > 0) => Ok(()),
        _ => bail!("argument `{key}` must be a positive integer"),
    }
}

fn expect_optional_string_array(args: &Value, key: &str) -> Result<()> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => Ok(()),
        _ => bail!("argument `{key}` must be an array of strings"),
    }
}
