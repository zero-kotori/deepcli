use super::{compact_text_line, required_arg, set_command_output_path, write_command_output};
use crate::privacy::redact_sensitive_text;
use crate::schema_ids;
use crate::session::SessionStore;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanCommandOptions {
    mode: PlanCommandMode,
    requirement: Option<String>,
    json_output: bool,
    output_path: Option<String>,
    write_doc: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanCommandMode {
    Show,
    Draft,
}

pub(crate) fn handle_plan_command(
    workspace: &Path,
    current: Option<String>,
    args: Vec<String>,
) -> Result<String> {
    let options = parse_plan_command_options(&args)?;
    match options.mode {
        PlanCommandMode::Show => {
            let Some(session_id) = current else {
                return Ok("no active plan".to_string());
            };
            let store = SessionStore::new(workspace);
            let session = store.load(&session_id)?;
            if let Some(plan) = session.load_plan()? {
                return Ok(serde_json::to_string_pretty(&plan)?);
            }
            Ok("no active plan".to_string())
        }
        PlanCommandMode::Draft => {
            let requirement = options
                .requirement
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("/plan requires a requirement or `show`"))?;
            let questions = planning_questions(requirement);
            let markdown = format_requirements_draft(requirement, &questions);
            let queued_questions = if let Some(session_id) = current {
                let store = SessionStore::new(workspace);
                let session = store.load(&session_id)?;
                let mut queued = Vec::new();
                for question in &questions {
                    let item = session.enqueue_side_question(format_planning_question(question))?;
                    queued.push(item.id.to_string());
                }
                queued
            } else {
                Vec::new()
            };
            if let Some(path) = &options.write_doc {
                write_command_output(workspace, path, &markdown)?;
            }
            let output = if options.json_output {
                format_plan_draft_json(
                    workspace,
                    requirement,
                    &questions,
                    &queued_questions,
                    &markdown,
                )?
            } else {
                markdown
            };
            if let Some(output_path) = &options.output_path {
                write_command_output(workspace, output_path, &output)?;
            }
            Ok(output)
        }
    }
}

fn parse_plan_command_options(args: &[String]) -> Result<PlanCommandOptions> {
    let mut mode = PlanCommandMode::Show;
    let mut requirement_parts = Vec::new();
    let mut json_output = false;
    let mut output_path = None;
    let mut write_doc = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "show" if requirement_parts.is_empty() => {
                mode = PlanCommandMode::Show;
                index += 1;
            }
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--output" | "-o" => {
                let raw = required_arg(args, index + 1, "output path")?;
                set_command_output_path(&mut output_path, raw)?;
                index += 2;
            }
            value if value.starts_with("--output=") => {
                set_command_output_path(&mut output_path, value.trim_start_matches("--output="))?;
                index += 1;
            }
            "--write-doc" | "--write-requirements" => {
                let raw = required_arg(args, index + 1, "requirements document path")?;
                set_command_output_path(&mut write_doc, raw)?;
                index += 2;
            }
            value if value.starts_with("--write-doc=") => {
                set_command_output_path(&mut write_doc, value.trim_start_matches("--write-doc="))?;
                index += 1;
            }
            value if value.starts_with('-') => bail!("unsupported /plan option `{value}`"),
            value => {
                if mode == PlanCommandMode::Show {
                    mode = PlanCommandMode::Draft;
                }
                requirement_parts.push(value.to_string());
                index += 1;
            }
        }
    }
    let requirement = (!requirement_parts.is_empty()).then(|| requirement_parts.join(" "));
    if mode == PlanCommandMode::Show && write_doc.is_some() {
        bail!("--write-doc requires a requirement");
    }
    Ok(PlanCommandOptions {
        mode,
        requirement,
        json_output,
        output_path,
        write_doc,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningQuestion {
    id: &'static str,
    question: String,
    options: Vec<PlanningOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningOption {
    label: &'static str,
    description: &'static str,
    recommended: bool,
}

fn planning_questions(requirement: &str) -> Vec<PlanningQuestion> {
    vec![
        PlanningQuestion {
            id: "scope",
            question: format!(
                "这次需求 `{}` 的首轮交付范围应如何收敛？",
                compact_text_line(requirement, 80)
            ),
            options: vec![
                PlanningOption {
                    label: "MVP first",
                    description: "先交付可验收主路径，保留清晰扩展点。",
                    recommended: true,
                },
                PlanningOption {
                    label: "Full feature",
                    description: "一次覆盖完整体验，耗时和风险更高。",
                    recommended: false,
                },
                PlanningOption {
                    label: "Research only",
                    description: "先只输出调研和方案，不修改产品。",
                    recommended: false,
                },
            ],
        },
        PlanningQuestion {
            id: "user",
            question: "主要用户是谁，优先优化哪类使用场景？".to_string(),
            options: vec![
                PlanningOption {
                    label: "Daily developer",
                    description: "面向日常编码、修复、测试和交付闭环。",
                    recommended: true,
                },
                PlanningOption {
                    label: "Maintainer",
                    description: "更重视审计、发布、CI 和团队治理。",
                    recommended: false,
                },
                PlanningOption {
                    label: "New user",
                    description: "更重视安装、引导和低学习成本。",
                    recommended: false,
                },
            ],
        },
        PlanningQuestion {
            id: "interaction",
            question: "交互方式优先放在哪里？".to_string(),
            options: vec![
                PlanningOption {
                    label: "TUI slash command",
                    description: "先在现有 TUI/CLI 命令体系内闭环。",
                    recommended: true,
                },
                PlanningOption {
                    label: "One-shot CLI",
                    description: "优先脚本化和 CI 调用。",
                    recommended: false,
                },
                PlanningOption {
                    label: "Both",
                    description: "同时做 TUI 和 one-shot，测试面更大。",
                    recommended: false,
                },
            ],
        },
        PlanningQuestion {
            id: "persistence",
            question: "需求产物需要保存到哪里？".to_string(),
            options: vec![
                PlanningOption {
                    label: "Session first",
                    description: "先写入当前 session，便于恢复和继续追问。",
                    recommended: true,
                },
                PlanningOption {
                    label: "Docs file",
                    description: "直接写入 docs/ai，便于提交和评审。",
                    recommended: false,
                },
                PlanningOption {
                    label: "Export only",
                    description: "只生成本地导出，不改变项目文档。",
                    recommended: false,
                },
            ],
        },
        PlanningQuestion {
            id: "acceptance",
            question: "验收标准应以什么为准？".to_string(),
            options: vec![
                PlanningOption {
                    label: "Tests and gates",
                    description: "以已有测试、preflight、gate 和文档要求为准。",
                    recommended: true,
                },
                PlanningOption {
                    label: "Manual UX",
                    description: "以人工体验流程为主，适合纯交互变化。",
                    recommended: false,
                },
                PlanningOption {
                    label: "Benchmark",
                    description: "以 benchmark/scorecard 变化为主。",
                    recommended: false,
                },
            ],
        },
    ]
}

fn format_planning_question(question: &PlanningQuestion) -> String {
    let options = question
        .options
        .iter()
        .map(|option| {
            format!(
                "- {}{}: {}",
                option.label,
                if option.recommended {
                    " (Recommended)"
                } else {
                    ""
                },
                option.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{}", question.question, options)
}

fn format_requirements_draft(requirement: &str, questions: &[PlanningQuestion]) -> String {
    let title = compact_text_line(requirement, 72);
    let mut lines = vec![
        format!("# Requirements Draft: {title}"),
        String::new(),
        "> Status: draft. Generated by `deepcli /plan`; answer the clarification questions before treating this as final.".to_string(),
        String::new(),
        "## Original Request".to_string(),
        String::new(),
        redact_sensitive_text(requirement),
        String::new(),
        "## Clarifying Questions".to_string(),
        String::new(),
    ];
    for (index, question) in questions.iter().enumerate() {
        lines.push(format!("{}. {}", index + 1, question.question));
        for option in &question.options {
            lines.push(format!(
                "   - {}{}: {}",
                option.label,
                if option.recommended {
                    " (Recommended)"
                } else {
                    ""
                },
                option.description
            ));
        }
    }
    lines.extend([
        String::new(),
        "## Working Assumptions".to_string(),
        String::new(),
        "- Prefer the smallest product increment that creates a usable, testable workflow.".to_string(),
        "- Keep behavior local-first, session-aware, and compatible with existing slash commands.".to_string(),
        "- Do not call a provider or modify unrelated files during planning unless explicitly requested.".to_string(),
        String::new(),
        "## Functional Requirements".to_string(),
        String::new(),
        "- The feature must be reachable from the CLI/TUI command surface.".to_string(),
        "- The feature must preserve existing session, permission, and output-path safety behavior.".to_string(),
        "- The feature must provide human-readable output and a stable JSON/report path when useful.".to_string(),
        "- The feature must make the next action obvious after partial or missing answers.".to_string(),
        String::new(),
        "## Acceptance Criteria".to_string(),
        String::new(),
        "- Help text and command discovery include the new workflow.".to_string(),
        "- Focused tests cover parsing, successful output, and at least one error/edge path.".to_string(),
        "- `cargo test` passes for the touched behavior.".to_string(),
        "- No credentials, local logs, benchmark artifacts, or machine-only evidence are committed.".to_string(),
        String::new(),
        "## Next Actions".to_string(),
        String::new(),
        "- Answer the clarification questions, then ask the agent to update this draft into a final requirements document.".to_string(),
        "- Use `/goal` before implementation if the work must continue until all documented acceptance gates pass.".to_string(),
    ]);
    lines.join("\n")
}

fn format_plan_draft_json(
    workspace: &Path,
    requirement: &str,
    questions: &[PlanningQuestion],
    queued_questions: &[String],
    markdown: &str,
) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "schema": schema_ids::PLAN_REQUIREMENTS_DRAFT_V1,
        "status": "draft",
        "workspace": workspace.display().to_string(),
        "requirement": redact_sensitive_text(requirement),
        "questions": questions.iter().map(planning_question_json).collect::<Vec<_>>(),
        "queuedSideQuestions": queued_questions,
        "recommendedNextActions": [
            "answer queued questions in the current session",
            "write the draft with `/plan <requirement> --write-doc docs/ai/REQUIREMENTS_DRAFT.md`",
            "start a strict implementation goal with `/goal <objective>`"
        ],
        "document": markdown,
    }))?)
}

fn planning_question_json(question: &PlanningQuestion) -> Value {
    json!({
        "id": question.id,
        "question": redact_sensitive_text(&question.question),
        "options": question.options.iter().map(|option| json!({
            "label": option.label,
            "description": option.description,
            "recommended": option.recommended,
        })).collect::<Vec<_>>(),
    })
}
