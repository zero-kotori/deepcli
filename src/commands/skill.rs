use super::{
    dedup_preserve_order, local_action_checklist, required_arg, set_command_output_path,
    write_command_output,
};
use crate::schema_ids;
use crate::skills::{LoadedSkill, SkillMetadata, SkillStore};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn handle_skill(workspace: &Path, args: Vec<String>) -> Result<String> {
    let store = SkillStore::new(workspace);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let option_args = if args.first().map(String::as_str) == Some("list") {
                &args[1..]
            } else {
                args.as_slice()
            };
            let options = parse_skill_read_options(option_args, "/skill list")?;
            let skills = store.discover()?;
            let text = if skills.is_empty() {
                "no project skills registered; create one with `/skill generate <name> <description>`"
                    .to_string()
            } else {
                skills
                    .iter()
                    .map(|skill| format!("{} - {}", skill.name, skill.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let output = if options.json_output {
                format_skill_list_json(workspace, &skills, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(value) if value.starts_with("--") => {
            let options = parse_skill_read_options(&args, "/skill list")?;
            let skills = store.discover()?;
            let text = if skills.is_empty() {
                "no project skills registered; create one with `/skill generate <name> <description>`"
                    .to_string()
            } else {
                skills
                    .iter()
                    .map(|skill| format!("{} - {}", skill.name, skill.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let output = if options.json_output {
                format_skill_list_json(workspace, &skills, &text)?
            } else {
                text
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some("generate") => {
            let name = required_arg(&args, 1, "skill name")?;
            let description = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            if description.trim().is_empty() {
                bail!("/skill generate requires a description");
            }
            Ok(store
                .generate(name, &description)?
                .instruction_path
                .display()
                .to_string())
        }
        Some("run") => {
            let name = required_arg(&args, 1, "skill name")?;
            let options = parse_skill_read_options(&args[2..], "/skill run")?;
            let loaded = store.load(name)?;
            let output = if options.json_output {
                format_skill_run_json(workspace, &loaded)?
            } else {
                loaded.instructions.clone()
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
        Some(other) => bail!("unsupported /skill action `{other}`"),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SkillReadOptions {
    json_output: bool,
    output_path: Option<String>,
}

fn parse_skill_read_options(args: &[String], command: &str) -> Result<SkillReadOptions> {
    let mut options = SkillReadOptions::default();
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

fn format_skill_list_json(
    workspace: &Path,
    skills: &[SkillMetadata],
    report: &str,
) -> Result<String> {
    let next_actions = skill_next_actions(
        skills.first().map(|skill| skill.name.as_str()),
        skills.is_empty(),
    );
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SKILL_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "list",
        "skillCount": skills.len(),
        "skills": skills
            .iter()
            .map(|skill| skill_metadata_json(workspace, skill))
            .collect::<Vec<_>>(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": report,
        "format": "json",
    }))?)
}

fn format_skill_run_json(workspace: &Path, loaded: &LoadedSkill) -> Result<String> {
    let next_actions = skill_next_actions(Some(&loaded.metadata.name), false);
    let checklist = local_action_checklist(&next_actions);
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::SKILL_INSPECT_V1,
        "status": "ok",
        "workspace": workspace.display().to_string(),
        "kind": "run",
        "skill": skill_metadata_json(workspace, &loaded.metadata),
        "instructions": loaded.instructions.as_str(),
        "instructionChars": loaded.instructions.chars().count(),
        "checklist": checklist,
        "nextActions": next_actions,
        "report": loaded.instructions.as_str(),
        "format": "json",
    }))?)
}

fn skill_metadata_json(workspace: &Path, skill: &SkillMetadata) -> Value {
    let skill_dir = workspace.join(".deepcli").join("skills").join(&skill.name);
    json!({
        "name": skill.name.as_str(),
        "description": skill.description.as_str(),
        "trigger": skill.trigger.as_str(),
        "maxDepth": skill.max_depth,
        "createdAt": skill.created_at.to_rfc3339(),
        "path": skill_dir.display().to_string(),
        "metadataPath": skill_dir.join("skill.json").display().to_string(),
        "instructionPath": skill_dir.join("SKILL.md").display().to_string(),
    })
}

fn skill_next_actions(name: Option<&str>, empty: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if empty {
        actions.push("deepcli help skill".to_string());
    } else if let Some(name) = name {
        actions.push(format!("deepcli skill run {name}"));
    } else {
        actions.push("deepcli skill list --json".to_string());
        actions.push("deepcli help skill".to_string());
    }
    if let Some(name) = name {
        actions.push(format!("deepcli skill run {name} --json"));
    }
    actions.push("deepcli skill list --json".to_string());
    dedup_preserve_order(actions)
}
