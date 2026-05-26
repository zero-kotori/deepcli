use crate::permissions::PermissionDecision;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    New,
    ContextLoading,
    WaitingUser,
    Planning,
    AwaitingApproval,
    Executing,
    Testing,
    Reviewing,
    Paused,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMetadata {
    pub id: Uuid,
    pub workspace: PathBuf,
    pub state: SessionState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub tool: String,
    pub input: Value,
    pub output: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<PermissionDecision>,
    pub status: ToolCallStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEvent {
    pub session_id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Requested,
    PolicyChecking,
    AutoApproved,
    UserApproved,
    Denied,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub status: PlanStepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestRunRecord {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub passed: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionActivitySummary {
    pub message_count: usize,
    pub tool_call_count: usize,
    pub test_run_count: usize,
    pub diff_count: usize,
    pub backup_count: usize,
    pub side_question_count: usize,
    pub approval_request_count: usize,
    pub has_summary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideQuestionStatus {
    Open,
    Answered,
    Cleared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SideQuestion {
    pub id: Uuid,
    pub question: String,
    pub answer: Option<String>,
    pub status: SideQuestionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Cleared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub tool: String,
    pub decision: PermissionDecision,
    pub status: ApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub metadata: SessionMetadata,
    path: PathBuf,
}

impl SessionStore {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            root: workspace.as_ref().join(".deepcli").join("sessions"),
        }
    }

    pub fn create(
        &self,
        workspace: impl AsRef<Path>,
        provider: String,
        model: Option<String>,
    ) -> Result<Session> {
        let now = Utc::now();
        let metadata = SessionMetadata {
            id: Uuid::new_v4(),
            workspace: workspace.as_ref().to_path_buf(),
            state: SessionState::New,
            created_at: now,
            updated_at: now,
            provider,
            model,
        };
        let session = Session {
            path: self.root.join(metadata.id.to_string()),
            metadata,
        };
        session.ensure_dirs()?;
        session.save_metadata()?;
        Ok(session)
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let path = self.root.join(id);
        let metadata_path = path.join("metadata.json");
        let raw = fs::read_to_string(&metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        let metadata = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
        Ok(Session { metadata, path })
    }

    pub fn list(&self) -> Result<Vec<SessionMetadata>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let metadata = entry.path().join("metadata.json");
            if metadata.exists() {
                let raw = fs::read_to_string(&metadata)?;
                sessions.push(serde_json::from_str(&raw)?);
            }
        }
        sessions.sort_by_key(|session: &SessionMetadata| session.updated_at);
        sessions.reverse();
        Ok(sessions)
    }
}

impl Session {
    pub fn id(&self) -> Uuid {
        self.metadata.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_state(&mut self, state: SessionState) -> Result<()> {
        self.metadata.state = state;
        self.metadata.updated_at = Utc::now();
        self.save_metadata()
    }

    pub fn append_message(&self, role: &str, content: impl Into<String>) -> Result<()> {
        self.append_jsonl(
            "messages.jsonl",
            &SessionMessage {
                role: role.to_string(),
                content: content.into(),
                created_at: Utc::now(),
            },
        )
    }

    pub fn append_tool_call(&self, record: &ToolCallRecord) -> Result<()> {
        self.append_jsonl("tools.jsonl", record)?;
        self.append_audit_event("tool_call", serde_json::to_value(record)?)
    }

    pub fn append_test_run(&self, record: &TestRunRecord) -> Result<()> {
        self.append_jsonl("tests.jsonl", record)?;
        self.append_audit_event("test_run", serde_json::to_value(record)?)
    }

    pub fn append_audit_event(&self, event_type: &str, payload: serde_json::Value) -> Result<()> {
        let event = AuditEvent {
            session_id: self.metadata.id,
            event_type: event_type.to_string(),
            payload,
            created_at: Utc::now(),
        };
        self.append_workspace_jsonl("logs/audit.jsonl", &event)
    }

    pub fn save_plan(&self, plan: &Plan) -> Result<()> {
        self.write_json("plan.json", plan)
    }

    pub fn load_plan(&self) -> Result<Option<Plan>> {
        self.read_json_if_exists("plan.json")
    }

    pub fn update_plan_step(&self, id: &str, status: PlanStepStatus) -> Result<()> {
        if let Some(mut plan) = self.load_plan()? {
            if let Some(step) = plan.steps.iter_mut().find(|step| step.id == id) {
                step.status = status;
                plan.updated_at = Utc::now();
                self.save_plan(&plan)?;
            }
        }
        Ok(())
    }

    pub fn complete_pending_plan_steps(&self) -> Result<()> {
        if let Some(mut plan) = self.load_plan()? {
            for step in &mut plan.steps {
                if step.status == PlanStepStatus::Pending
                    || step.status == PlanStepStatus::InProgress
                {
                    step.status = PlanStepStatus::Completed;
                }
            }
            plan.updated_at = Utc::now();
            self.save_plan(&plan)?;
        }
        Ok(())
    }

    pub fn activity_summary(&self) -> Result<SessionActivitySummary> {
        Ok(SessionActivitySummary {
            message_count: count_jsonl_lines(&self.path.join("messages.jsonl"))?,
            tool_call_count: count_jsonl_lines(&self.path.join("tools.jsonl"))?,
            test_run_count: count_jsonl_lines(&self.path.join("tests.jsonl"))?,
            diff_count: count_files(&self.path.join("diffs"))?,
            backup_count: count_files(&self.path.join("backups"))?,
            side_question_count: self.load_side_questions()?.len(),
            approval_request_count: self.load_approval_requests()?.len(),
            has_summary: self.path.join("summary.md").exists(),
        })
    }

    pub fn enqueue_side_question(&self, question: impl Into<String>) -> Result<SideQuestion> {
        let question = question.into();
        let now = Utc::now();
        let item = SideQuestion {
            id: Uuid::new_v4(),
            question,
            answer: None,
            status: SideQuestionStatus::Open,
            created_at: now,
            updated_at: now,
        };
        let mut items = self.load_side_questions()?;
        items.push(item.clone());
        self.save_side_questions(&items)?;
        self.append_audit_event("side_question_queued", serde_json::to_value(&item)?)?;
        Ok(item)
    }

    pub fn load_side_questions(&self) -> Result<Vec<SideQuestion>> {
        Ok(self
            .read_json_if_exists("side_questions.json")?
            .unwrap_or_default())
    }

    pub fn answer_side_question(
        &self,
        id: &str,
        answer: impl Into<String>,
    ) -> Result<SideQuestion> {
        let mut items = self.load_side_questions()?;
        let index = resolve_side_question_index(&items, id)?;
        let now = Utc::now();
        items[index].answer = Some(answer.into());
        items[index].status = SideQuestionStatus::Answered;
        items[index].updated_at = now;
        let updated = items[index].clone();
        self.save_side_questions(&items)?;
        self.append_audit_event("side_question_answered", serde_json::to_value(&updated)?)?;
        Ok(updated)
    }

    pub fn clear_side_questions(&self) -> Result<usize> {
        let mut items = self.load_side_questions()?;
        let now = Utc::now();
        let mut changed = 0;
        for item in &mut items {
            if item.status == SideQuestionStatus::Open {
                item.status = SideQuestionStatus::Cleared;
                item.updated_at = now;
                changed += 1;
            }
        }
        self.save_side_questions(&items)?;
        self.append_audit_event(
            "side_questions_cleared",
            serde_json::json!({ "cleared": changed }),
        )?;
        Ok(changed)
    }

    pub fn enqueue_approval_request(
        &self,
        tool: impl Into<String>,
        decision: PermissionDecision,
    ) -> Result<ApprovalRequest> {
        let now = Utc::now();
        let request = ApprovalRequest {
            id: Uuid::new_v4(),
            tool: tool.into(),
            decision,
            status: ApprovalStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        let mut items = self.load_approval_requests()?;
        items.push(request.clone());
        self.save_approval_requests(&items)?;
        self.append_audit_event("approval_requested", serde_json::to_value(&request)?)?;
        Ok(request)
    }

    pub fn load_approval_requests(&self) -> Result<Vec<ApprovalRequest>> {
        Ok(self
            .read_json_if_exists("approvals.json")?
            .unwrap_or_default())
    }

    pub fn update_approval_request(
        &self,
        id: &str,
        status: ApprovalStatus,
    ) -> Result<ApprovalRequest> {
        let mut items = self.load_approval_requests()?;
        let index = resolve_approval_request_index(&items, id)?;
        items[index].status = status;
        items[index].updated_at = Utc::now();
        let updated = items[index].clone();
        self.save_approval_requests(&items)?;
        self.append_audit_event("approval_updated", serde_json::to_value(&updated)?)?;
        Ok(updated)
    }

    pub fn clear_pending_approval_requests(&self) -> Result<usize> {
        let mut items = self.load_approval_requests()?;
        let now = Utc::now();
        let mut changed = 0;
        for item in &mut items {
            if item.status == ApprovalStatus::Pending {
                item.status = ApprovalStatus::Cleared;
                item.updated_at = now;
                changed += 1;
            }
        }
        self.save_approval_requests(&items)?;
        self.append_audit_event(
            "approvals_cleared",
            serde_json::json!({ "cleared": changed }),
        )?;
        Ok(changed)
    }

    pub fn save_diff(&self, name: &str, diff: &str) -> Result<PathBuf> {
        let diffs = self.path.join("diffs");
        fs::create_dir_all(&diffs)?;
        let safe_name = name.replace('/', "_");
        let path = diffs.join(format!("{safe_name}.diff"));
        fs::write(&path, diff)?;
        Ok(path)
    }

    pub fn save_backup(&self, name: &str, content: &str) -> Result<PathBuf> {
        let backups = self.path.join("backups");
        fs::create_dir_all(&backups)?;
        let safe_name = name.replace('/', "_");
        let path = backups.join(format!("{safe_name}.bak"));
        fs::write(&path, content)?;
        Ok(path)
    }

    pub fn write_summary(&self, summary: &str) -> Result<()> {
        fs::write(self.path.join("summary.md"), summary)
            .with_context(|| format!("failed to write summary for {}", self.metadata.id))
    }

    fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.path.join("diffs"))
            .with_context(|| format!("failed to create {}", self.path.display()))
    }

    fn save_metadata(&self) -> Result<()> {
        self.write_json("metadata.json", &self.metadata)
    }

    fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        self.ensure_dirs()?;
        let path = self.path.join(name);
        fs::write(&path, serde_json::to_vec_pretty(value)?)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    fn read_json_if_exists<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let path = self.path.join(name);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    fn append_jsonl<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        self.ensure_dirs()?;
        let path = self.path.join(name);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        serde_json::to_writer(&mut file, value)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    fn append_workspace_jsonl<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let path = self.metadata.workspace.join(".deepcli").join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        serde_json::to_writer(&mut file, value)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    fn save_side_questions(&self, items: &[SideQuestion]) -> Result<()> {
        self.write_json("side_questions.json", &items)
    }

    fn save_approval_requests(&self, items: &[ApprovalRequest]) -> Result<()> {
        self.write_json("approvals.json", &items)
    }
}

fn resolve_side_question_index(items: &[SideQuestion], id: &str) -> Result<usize> {
    let matches = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.id.to_string().starts_with(id))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => anyhow::bail!("side question `{id}` not found"),
        _ => anyhow::bail!("side question id prefix `{id}` is ambiguous"),
    }
}

fn resolve_approval_request_index(items: &[ApprovalRequest], id: &str) -> Result<usize> {
    let matches = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.id.to_string().starts_with(id))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => anyhow::bail!("approval request `{id}` not found"),
        _ => anyhow::bail!("approval request id prefix `{id}` is ambiguous"),
    }
}

fn count_jsonl_lines(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(raw.lines().filter(|line| !line.trim().is_empty()).count())
}

fn count_files(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(path)? {
        if entry?.file_type()?.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stores_session_messages_and_plan() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("model".to_string()),
            )
            .unwrap();
        session.append_message("user", "hello").unwrap();
        session
            .append_tool_call(&ToolCallRecord {
                tool: "read_file".to_string(),
                input: serde_json::json!({"path": "Cargo.toml"}),
                output: serde_json::json!({"ok": true}),
                decision: None,
                status: ToolCallStatus::Succeeded,
                created_at: Utc::now(),
            })
            .unwrap();
        session
            .save_plan(&Plan {
                title: "test".to_string(),
                steps: vec![PlanStep {
                    id: "1".to_string(),
                    description: "step".to_string(),
                    status: PlanStepStatus::Pending,
                }],
                updated_at: Utc::now(),
            })
            .unwrap();

        let loaded = store.load(&session.id().to_string()).unwrap();
        assert_eq!(loaded.metadata.provider, "deepseek");
        assert_eq!(loaded.load_plan().unwrap().unwrap().steps.len(), 1);
        assert!(loaded.path().join("messages.jsonl").exists());
        assert!(dir.path().join(".deepcli/logs/audit.jsonl").exists());
        assert_eq!(loaded.activity_summary().unwrap().message_count, 1);
        assert_eq!(loaded.activity_summary().unwrap().side_question_count, 0);
        assert_eq!(loaded.activity_summary().unwrap().approval_request_count, 0);
        loaded
            .update_plan_step("1", PlanStepStatus::Completed)
            .unwrap();
        assert_eq!(
            loaded.load_plan().unwrap().unwrap().steps[0].status,
            PlanStepStatus::Completed
        );
    }

    #[test]
    fn stores_side_questions_with_answer_and_clear_states() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let first = session
            .enqueue_side_question("explain the diff later")
            .unwrap();
        let second = session.enqueue_side_question("what tests ran").unwrap();

        let answered = session
            .answer_side_question(&first.id.to_string()[..8], "after the main task")
            .unwrap();
        assert_eq!(answered.status, SideQuestionStatus::Answered);
        assert_eq!(answered.answer.as_deref(), Some("after the main task"));

        assert_eq!(session.clear_side_questions().unwrap(), 1);
        let questions = session.load_side_questions().unwrap();
        assert_eq!(questions.len(), 2);
        assert!(questions
            .iter()
            .any(|item| item.id == first.id && item.status == SideQuestionStatus::Answered));
        assert!(questions
            .iter()
            .any(|item| item.id == second.id && item.status == SideQuestionStatus::Cleared));
        assert_eq!(session.activity_summary().unwrap().side_question_count, 2);
    }

    #[test]
    fn stores_approval_requests_with_status_updates() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let request = session
            .enqueue_approval_request(
                "write_file",
                PermissionDecision {
                    outcome: crate::permissions::DecisionOutcome::RequiresUserApproval,
                    risk: crate::permissions::RiskLevel::High,
                    reason: "write requires approval".to_string(),
                },
            )
            .unwrap();

        let approved = session
            .update_approval_request(&request.id.to_string()[..8], ApprovalStatus::Approved)
            .unwrap();
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert_eq!(session.load_approval_requests().unwrap().len(), 1);
        assert_eq!(
            session.activity_summary().unwrap().approval_request_count,
            1
        );
    }
}
