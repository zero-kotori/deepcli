use super::{
    compact_text_line, dedup_preserve_order, local_action_checklist, parse_positive_usize,
    required_arg, set_command_output_path, write_command_output,
};
use crate::prompts::{Prompt, PromptStore};
use crate::tools::ToolExecutor;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) async fn handle_prompt(
    workspace: &Path,
    executor: &ToolExecutor,
    args: Vec<String>,
) -> Result<String> {
    let store = PromptStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let option_args = if args.first().map(String::as_str) == Some("list") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_prompt_read_options(option_args, "/prompt list")?;
            let prompts = store.list()?;
            let text = prompts
                .iter()
                .map(|prompt| format!("{} - {}", prompt.name, prompt.description))
                .collect::<Vec<_>>()
                .join("\n");
            let output = if options.json_output {
                format_prompt_list_json(workspace, &prompts, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_prompt_read_options(&args, "/prompt list")?;
            let prompts = store.list()?;
            let text = prompts
                .iter()
                .map(|prompt| format!("{} - {}", prompt.name, prompt.description))
                .collect::<Vec<_>>()
                .join("\n");
            let output = if options.json_output {
                format_prompt_list_json(workspace, &prompts, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("get") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let options = parse_prompt_read_options(&args[2..], "/prompt get")?;
            let prompt = store.get(name)?;
            let output = if options.json_output {
                format_prompt_get_json(workspace, &prompt)?
            } else {
                prompt.body.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("render") => {
            let (render_command_args, options) = split_prompt_render_options(&args)?;
            let render_args = parse_prompt_render_args(&render_command_args)?;
            let execution = executor.execute("prompt_render", render_args).await?;
            let output = if options.json_output {
                format_prompt_render_json(workspace, &execution.raw, &execution.content)?
            } else {
                execution.content
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("save") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let body = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if body.trim().is_empty() {
                bail!("/prompt save requires a body");
            }
            let path = store.save(name, &body)?;
            Ok(path.display().to_string())
        }
        Some("delete") | Some("rm") => {
            let name = required_arg(&args, 1, "prompt name")?;
            let path = store.delete(name)?;
            Ok(format!("deleted prompt `{name}` at {}", path.display()))
        }
        Some(other) => bail!("unsupported /prompt action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PromptReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_prompt_read_options(args: &[String], command: &str) -> Result<PromptReadOptions> {
    let mut options = PromptReadOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("{command} --output requires a path"))?;
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
            value => bail!("unsupported {command} option `{value}`"),
        }
    }
    Ok(options)
}

fn split_prompt_render_options(args: &[String]) -> Result<(Vec<String>, PromptReadOptions)> {
    let mut render_args = Vec::new();
    let mut options = PromptReadOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json_output = true;
                index += 1;
            }
            "--output" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("/prompt render --output requires a path"))?;
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
            value => {
                render_args.push(value.to_string());
                index += 1;
            }
        }
    }
    Ok((render_args, options))
}

fn format_prompt_list_json(workspace: &Path, prompts: &[Prompt], report: &str) -> Result<String> {
    let next_actions = prompt_next_actions(prompts.first().map(|prompt| prompt.name.as_str()));
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.prompt.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "promptCount": prompts.len(),
        "prompts": prompts
            .iter()
            .map(|prompt| prompt_summary_json(workspace, prompt))
            .collect::<Vec<_>>(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
        "format": "json",
    }))?)
}

fn format_prompt_get_json(workspace: &Path, prompt: &Prompt) -> Result<String> {
    let next_actions = prompt_next_actions(Some(&prompt.name));
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.prompt.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "get",
        "prompt": prompt_detail_json(workspace, prompt),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": prompt.body.as_str(),
        "format": "json",
    }))?)
}

fn format_prompt_render_json(workspace: &Path, raw: &Value, rendered: &str) -> Result<String> {
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let next_actions = prompt_next_actions(Some(name));
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": "deepcli.prompt.inspect.v1",
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "render",
        "prompt": {
            "name": name,
            "description": raw.get("description").and_then(Value::as_str).unwrap_or(""),
        },
        "context": raw.get("context").cloned().unwrap_or(Value::Null),
        "rendered": rendered,
        "renderedChars": rendered.chars().count(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": rendered,
        "format": "json",
    }))?)
}

fn prompt_summary_json(workspace: &Path, prompt: &Prompt) -> Value {
    let source = prompt_source(workspace, &prompt.name);
    json!({
        "name": prompt.name.as_str(),
        "description": prompt.description.as_str(),
        "source": source,
        "path": prompt_path_json(workspace, &prompt.name, source),
        "bodyChars": prompt.body.chars().count(),
        "bodyPreview": compact_text_line(&prompt.body, 160),
    })
}

fn prompt_detail_json(workspace: &Path, prompt: &Prompt) -> Value {
    let source = prompt_source(workspace, &prompt.name);
    json!({
        "name": prompt.name.as_str(),
        "description": prompt.description.as_str(),
        "source": source,
        "path": prompt_path_json(workspace, &prompt.name, source),
        "body": prompt.body.as_str(),
        "bodyChars": prompt.body.chars().count(),
    })
}

fn prompt_source(workspace: &Path, name: &str) -> &'static str {
    if workspace
        .join(".deepcli")
        .join("prompts")
        .join(format!("{name}.md"))
        .exists()
    {
        "custom"
    } else {
        "builtin"
    }
}

fn prompt_path_json(workspace: &Path, name: &str, source: &str) -> Value {
    if source == "custom" {
        json!(workspace
            .join(".deepcli")
            .join("prompts")
            .join(format!("{name}.md"))
            .display()
            .to_string())
    } else {
        Value::Null
    }
}

fn prompt_next_actions(name: Option<&str>) -> Vec<String> {
    let mut actions = Vec::new();
    if let Some(name) = name {
        actions.push(format!("deepcli prompt get {name}"));
        actions.push(format!("deepcli prompt render {name} --json"));
    } else {
        actions.push("deepcli prompt list --json".to_string());
    }
    actions.push("deepcli help prompt".to_string());
    dedup_preserve_order(actions)
}

fn parse_prompt_render_args(args: &[String]) -> Result<Value> {
    let name = required_arg(args, 1, "prompt name")?;
    let mut file = None;
    let mut max_diff_chars = None;
    let mut max_file_chars = None;
    let mut variables = serde_json::Map::new();
    let mut index = 2;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--file" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("/prompt render --file requires a path"))?;
            file = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--file=") {
            file = Some(value.to_string());
        } else if arg == "--max-diff-chars" {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                anyhow::anyhow!("/prompt render --max-diff-chars requires a number")
            })?;
            max_diff_chars = Some(parse_positive_usize(value, "--max-diff-chars")?);
        } else if let Some(value) = arg.strip_prefix("--max-diff-chars=") {
            max_diff_chars = Some(parse_positive_usize(value, "--max-diff-chars")?);
        } else if arg == "--max-file-chars" {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                anyhow::anyhow!("/prompt render --max-file-chars requires a number")
            })?;
            max_file_chars = Some(parse_positive_usize(value, "--max-file-chars")?);
        } else if let Some(value) = arg.strip_prefix("--max-file-chars=") {
            max_file_chars = Some(parse_positive_usize(value, "--max-file-chars")?);
        } else if let Some((key, value)) = arg.split_once('=') {
            if key.trim().is_empty() {
                bail!("/prompt render variable name cannot be empty");
            }
            variables.insert(key.to_string(), Value::String(value.to_string()));
        } else {
            bail!("unsupported /prompt render argument `{arg}`");
        }
        index += 1;
    }

    let mut value = json!({"name": name});
    if let Some(file) = file {
        value["file"] = Value::String(file);
    }
    if let Some(max_diff_chars) = max_diff_chars {
        value["max_diff_chars"] = json!(max_diff_chars);
    }
    if let Some(max_file_chars) = max_file_chars {
        value["max_file_chars"] = json!(max_file_chars);
    }
    if !variables.is_empty() {
        value["variables"] = Value::Object(variables);
    }
    Ok(value)
}
