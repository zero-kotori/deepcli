use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Queued,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentTask {
    pub id: Uuid,
    pub parent_session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<Uuid>,
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
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentEvent {
    pub timestamp: DateTime<Utc>,
    pub task_id: Uuid,
    #[serde(rename = "type")]
    pub event_type: String,
    pub status: SubagentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
    event_root: PathBuf,
    log_root: PathBuf,
    workspace: PathBuf,
}

impl AgentStore {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        let workspace = workspace.as_ref().to_path_buf();
        let agent_root = workspace.join(".deepcli").join("agents");
        Self {
            root: agent_root.join("tasks"),
            event_root: agent_root.join("events"),
            log_root: agent_root.join("logs"),
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
        let id = Uuid::new_v4();
        let task = SubagentTask {
            id,
            parent_session_id: options.parent_session_id,
            child_session_id: None,
            task: options.task,
            depth: options.depth,
            read_scope: normalized_read_scope,
            write_scope: normalized_write_scope,
            allowed_tools: options.allowed_tools,
            context: options.context,
            status: SubagentStatus::Queued,
            attempts: 0,
            pid: None,
            event_log_path: Some(subagent_log_relative_path(id, "events", "jsonl")),
            output_log_path: Some(subagent_log_relative_path(id, "logs", "log")),
            started_at: None,
            heartbeat_at: None,
            completed_at: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        };
        self.save(&task)?;
        self.append_subagent_event(&task, "created", None)?;
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

    pub fn subagent_event_log_path(&self, id: Uuid) -> PathBuf {
        self.event_root.join(format!("{id}.jsonl"))
    }

    pub fn subagent_output_log_path(&self, id: Uuid) -> PathBuf {
        self.log_root.join(format!("{id}.log"))
    }

    pub fn read_subagent_events(&self, id: Uuid) -> Result<Vec<SubagentEvent>> {
        let path = self.subagent_event_log_path(id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let mut events = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            events.push(
                serde_json::from_str::<SubagentEvent>(&line)
                    .with_context(|| format!("failed to parse {}", path.display()))?,
            );
        }
        Ok(events)
    }

    pub fn record_subagent_background_process(
        &self,
        id: Uuid,
        pid: Option<u32>,
    ) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        let now = Utc::now();
        task.status = SubagentStatus::Running;
        task.pid = pid;
        task.heartbeat_at = Some(now);
        task.updated_at = now;
        self.save(&task)?;
        self.append_subagent_event(
            &task,
            "background_started",
            Some("background process spawned"),
        )?;
        Ok(task)
    }

    pub fn record_subagent_scheduled(&self, id: Uuid, reason: &str) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        task.updated_at = Utc::now();
        self.save(&task)?;
        self.append_subagent_event(&task, "scheduled", Some(reason))?;
        Ok(task)
    }

    pub fn mark_subagent_started(
        &self,
        id: Uuid,
        child_session_id: Option<Uuid>,
        pid: Option<u32>,
    ) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        let now = Utc::now();
        task.status = SubagentStatus::Running;
        task.child_session_id = child_session_id.or(task.child_session_id);
        task.pid = pid.or(task.pid);
        task.attempts = task.attempts.saturating_add(1);
        task.started_at = Some(now);
        task.heartbeat_at = Some(now);
        task.completed_at = None;
        task.last_error = None;
        task.updated_at = now;
        self.save(&task)?;
        self.append_subagent_event(&task, "started", None)?;
        Ok(task)
    }

    pub fn heartbeat_subagent(&self, id: Uuid) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        let now = Utc::now();
        task.heartbeat_at = Some(now);
        task.updated_at = now;
        self.save(&task)?;
        self.append_subagent_event(&task, "heartbeat", None)?;
        Ok(task)
    }

    pub fn complete_subagent(&self, id: Uuid, summary: &str) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        let now = Utc::now();
        task.status = SubagentStatus::Completed;
        task.pid = None;
        task.completed_at = Some(now);
        task.heartbeat_at = Some(now);
        task.last_error = None;
        task.updated_at = now;
        self.save(&task)?;
        self.append_subagent_event(&task, "completed", Some(summary))?;
        Ok(task)
    }

    pub fn await_subagent_approval(&self, id: Uuid, summary: &str) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        let now = Utc::now();
        task.status = SubagentStatus::AwaitingApproval;
        task.pid = None;
        task.completed_at = None;
        task.heartbeat_at = Some(now);
        task.last_error = None;
        task.updated_at = now;
        self.save(&task)?;
        self.append_subagent_event(&task, "awaiting_approval", Some(summary))?;
        Ok(task)
    }

    pub fn fail_subagent(&self, id: Uuid, error: &str) -> Result<SubagentTask> {
        let mut task = self.load(id)?;
        let now = Utc::now();
        task.status = SubagentStatus::Failed;
        task.pid = None;
        task.completed_at = Some(now);
        task.heartbeat_at = Some(now);
        task.last_error = Some(error.to_string());
        task.updated_at = now;
        self.save(&task)?;
        self.append_subagent_event(&task, "failed", Some(error))?;
        Ok(task)
    }

    fn append_subagent_event(
        &self,
        task: &SubagentTask,
        event_type: &str,
        message: Option<&str>,
    ) -> Result<()> {
        fs::create_dir_all(&self.event_root)
            .with_context(|| format!("failed to create {}", self.event_root.display()))?;
        let path = self.subagent_event_log_path(task.id);
        let event = SubagentEvent {
            timestamp: Utc::now(),
            task_id: task.id,
            event_type: event_type.to_string(),
            status: task.status.clone(),
            child_session_id: task.child_session_id,
            pid: task.pid,
            message: message.map(str::to_string),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to write {}", path.display()))?;
        writeln!(file, "{}", serde_json::to_string(&event)?)?;
        Ok(())
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

fn subagent_log_relative_path(id: Uuid, kind: &str, extension: &str) -> PathBuf {
    PathBuf::from(".deepcli")
        .join("agents")
        .join(kind)
        .join(format!("{id}.{extension}"))
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

    #[test]
    fn subagent_lifecycle_writes_events_and_resume_metadata() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        let task = store
            .create_subagent_task(None, "inspect parser", 1, vec![PathBuf::from("src")])
            .unwrap();
        let child_session_id = Uuid::new_v4();

        let started = store
            .mark_subagent_started(task.id, Some(child_session_id), Some(4242))
            .unwrap();
        assert_eq!(started.status, SubagentStatus::Running);
        assert_eq!(started.child_session_id, Some(child_session_id));
        assert_eq!(started.pid, Some(4242));
        assert_eq!(started.attempts, 1);
        assert!(started.started_at.is_some());
        assert!(started.heartbeat_at.is_some());

        store.heartbeat_subagent(task.id).unwrap();
        let completed = store
            .complete_subagent(task.id, "inspected parser successfully")
            .unwrap();
        assert_eq!(completed.status, SubagentStatus::Completed);
        assert_eq!(completed.child_session_id, Some(child_session_id));
        assert_eq!(completed.pid, None);
        assert!(completed.completed_at.is_some());
        assert_eq!(completed.last_error, None);

        let events = store.read_subagent_events(task.id).unwrap();
        let event_types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            event_types,
            vec!["created", "started", "heartbeat", "completed"]
        );
        assert!(store.subagent_event_log_path(task.id).exists());
        assert!(store
            .subagent_output_log_path(task.id)
            .ends_with(format!("{}.log", task.id)));
    }

    #[test]
    fn failed_subagent_can_be_marked_for_resume() {
        let dir = tempdir().unwrap();
        let store = AgentStore::new(dir.path());
        let task = store
            .create_subagent_task(None, "inspect parser", 1, Vec::new())
            .unwrap();
        let child_session_id = Uuid::new_v4();
        store
            .mark_subagent_started(task.id, Some(child_session_id), Some(7))
            .unwrap();

        let failed = store
            .fail_subagent(task.id, "DeepSeek apiKey is missing")
            .unwrap();

        assert_eq!(failed.status, SubagentStatus::Failed);
        assert_eq!(failed.child_session_id, Some(child_session_id));
        assert_eq!(failed.pid, None);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("DeepSeek apiKey is missing")
        );
        let events = store.read_subagent_events(task.id).unwrap();
        assert_eq!(events.last().unwrap().event_type, "failed");
    }
}
