use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentTask {
    pub id: Uuid,
    pub parent_session_id: Option<Uuid>,
    pub task: String,
    pub depth: u8,
    #[serde(default)]
    pub read_scope: Vec<PathBuf>,
    pub write_scope: Vec<PathBuf>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub status: SubagentStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SubagentTaskOptions {
    pub parent_session_id: Option<Uuid>,
    pub task: String,
    pub depth: u8,
    pub read_scope: Vec<PathBuf>,
    pub write_scope: Vec<PathBuf>,
    pub allowed_tools: Vec<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubagentTaskUpdate {
    pub task: String,
    pub read_scope: Vec<PathBuf>,
    pub write_scope: Vec<PathBuf>,
    pub allowed_tools: Vec<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentStore {
    root: PathBuf,
    workspace: PathBuf,
}

impl AgentStore {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        let workspace = workspace.as_ref().to_path_buf();
        Self {
            root: workspace.join(".deepcli").join("agents").join("tasks"),
            workspace,
        }
    }

    pub fn create_subagent_task(
        &self,
        parent_session_id: Option<Uuid>,
        task: &str,
        depth: u8,
        write_scope: Vec<PathBuf>,
    ) -> Result<SubagentTask> {
        self.create_subagent_task_with_options(SubagentTaskOptions {
            parent_session_id,
            task: task.to_string(),
            depth,
            read_scope: Vec::new(),
            write_scope,
            allowed_tools: Vec::new(),
            context: None,
        })
    }

    pub fn create_subagent_task_with_options(
        &self,
        options: SubagentTaskOptions,
    ) -> Result<SubagentTask> {
        if options.task.trim().is_empty() {
            bail!("sub-agent task cannot be empty");
        }
        let normalized_read_scope = options
            .read_scope
            .into_iter()
            .map(|path| normalize_scope_path(&self.workspace, &path))
            .collect::<Result<Vec<_>>>()?;
        let normalized_write_scope = options
            .write_scope
            .into_iter()
            .map(|path| normalize_scope_path(&self.workspace, &path))
            .collect::<Result<Vec<_>>>()?;
        let now = Utc::now();
        let task = SubagentTask {
            id: Uuid::new_v4(),
            parent_session_id: options.parent_session_id,
            task: options.task,
            depth: options.depth,
            read_scope: normalized_read_scope,
            write_scope: normalized_write_scope,
            allowed_tools: options.allowed_tools,
            context: options.context,
            status: SubagentStatus::Queued,
            created_at: now,
            updated_at: now,
        };
        self.save(&task)?;
        Ok(task)
    }

    pub fn save(&self, task: &SubagentTask) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let path = self.root.join(format!("{}.json", task.id));
        fs::write(&path, serde_json::to_vec_pretty(task)?)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn load(&self, id: Uuid) -> Result<SubagentTask> {
        let path = self.root.join(format!("{id}.json"));
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn list(&self) -> Result<Vec<SubagentTask>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut tasks = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let raw = fs::read_to_string(entry.path())?;
                tasks.push(serde_json::from_str(&raw)?);
            }
        }
        tasks.sort_by_key(|task: &SubagentTask| task.created_at);
        Ok(tasks)
    }

    pub fn update_queued_subagent_task(
        &self,
        id: Uuid,
        update: SubagentTaskUpdate,
    ) -> Result<SubagentTask> {
        if update.task.trim().is_empty() {
            bail!("sub-agent task cannot be empty");
        }
        let mut task = self.load(id)?;
        if task.status != SubagentStatus::Queued {
            bail!("only queued sub-agent tasks can be edited");
        }
        task.task = update.task;
        task.read_scope = update
            .read_scope
            .into_iter()
            .map(|path| normalize_scope_path(&self.workspace, &path))
            .collect::<Result<Vec<_>>>()?;
        task.write_scope = update
            .write_scope
            .into_iter()
            .map(|path| normalize_scope_path(&self.workspace, &path))
            .collect::<Result<Vec<_>>>()?;
        task.allowed_tools = update.allowed_tools;
        task.context = update.context;
        task.updated_at = Utc::now();
        self.save(&task)?;
        Ok(task)
    }
}

fn normalize_scope_path(workspace: &Path, raw: &Path) -> Result<PathBuf> {
    if raw
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!(
            "sub-agent write scope cannot contain parent traversal: {}",
            raw.display()
        );
    }
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace.join(raw)
    };
    if !absolute.starts_with(workspace) {
        bail!(
            "sub-agent write scope must stay inside workspace: {}",
            raw.display()
        );
    }
    Ok(absolute
        .strip_prefix(workspace)
        .unwrap_or(&absolute)
        .to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_and_lists_subagent_tasks() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        let task = store
            .create_subagent_task(
                None,
                "inspect parser",
                1,
                vec![PathBuf::from("src/parser.rs")],
            )
            .unwrap();
        let loaded = store.load(task.id).unwrap();
        assert_eq!(loaded.task, "inspect parser");
        assert_eq!(loaded.write_scope, vec![PathBuf::from("src/parser.rs")]);
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn rejects_subagent_scope_traversal() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        assert!(store
            .create_subagent_task(None, "bad", 1, vec![PathBuf::from("../outside")])
            .is_err());
    }

    #[test]
    fn updates_queued_subagent_task_descriptor() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        let task = store
            .create_subagent_task(
                None,
                "inspect parser",
                1,
                vec![PathBuf::from("src/parser.rs")],
            )
            .unwrap();

        let updated = store
            .update_queued_subagent_task(
                task.id,
                SubagentTaskUpdate {
                    task: "inspect lexer".to_string(),
                    read_scope: vec![PathBuf::from("README.md")],
                    write_scope: vec![PathBuf::from("src/lexer.rs")],
                    allowed_tools: vec!["read_file".to_string()],
                    context: Some("focus tokenization".to_string()),
                },
            )
            .unwrap();

        assert_eq!(updated.task, "inspect lexer");
        assert_eq!(updated.read_scope, vec![PathBuf::from("README.md")]);
        assert_eq!(updated.write_scope, vec![PathBuf::from("src/lexer.rs")]);
        assert_eq!(updated.allowed_tools, vec!["read_file"]);
        assert_eq!(updated.context.as_deref(), Some("focus tokenization"));
    }
}
