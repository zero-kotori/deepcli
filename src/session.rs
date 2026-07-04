use crate::permissions::PermissionDecision;
use crate::privacy::redact_sensitive_text;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderTranscriptToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderTranscriptRecord {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderTranscriptToolCall>,
    #[serde(default)]
    pub synthetic: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactBoundaryRecord {
    pub id: Uuid,
    pub reason: String,
    pub summary: String,
    pub omitted_group_count: usize,
    pub message_count_before: usize,
    pub message_count_after: usize,
    pub retained_segment: Vec<ProviderTranscriptRecord>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileHistorySnapshot {
    pub tool: String,
    pub target: String,
    pub summary: String,
    pub data: Value,
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
pub struct GoalContract {
    pub objective: String,
    pub source_requirements: Vec<String>,
    pub stop_conditions: Vec<String>,
    pub acceptance_commands: Vec<String>,
    pub status: GoalStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Complete,
    Blocked,
    Cancelled,
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
pub struct SessionDiffRecord {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBackupRecord {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<PathBuf>,
    pub content: String,
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SessionBackupIndexRecord {
    pub name: String,
    pub path: PathBuf,
    pub target_path: PathBuf,
    pub created_at: DateTime<Utc>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
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
            title: None,
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
        let id = self.resolve_id(id)?;
        let path = self.root.join(id);
        let metadata_path = path.join("metadata.json");
        let raw = fs::read_to_string(&metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        let metadata = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
        Ok(Session { metadata, path })
    }

    pub fn resolve_id(&self, selector: &str) -> Result<String> {
        let selector = selector.trim();
        if selector.is_empty() {
            anyhow::bail!("session id cannot be empty");
        }
        if selector.contains('/') || selector.contains('\\') || selector.contains("..") {
            anyhow::bail!("session id must be a full UUID or unique UUID prefix");
        }

        let exact = self.root.join(selector).join("metadata.json");
        if exact.exists() {
            return Ok(selector.to_string());
        }

        let matches = self
            .list()?
            .into_iter()
            .filter_map(|metadata| {
                let id = metadata.id.to_string();
                id.starts_with(selector).then_some(id)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [id] => Ok(id.clone()),
            [] => anyhow::bail!("session `{selector}` was not found"),
            _ => anyhow::bail!(
                "session id prefix `{selector}` is ambiguous; matched {} session(s)",
                matches.len()
            ),
        }
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

    pub fn rename(&mut self, title: impl Into<String>) -> Result<()> {
        let title = title.into().trim().to_string();
        self.metadata.title = if title.is_empty() { None } else { Some(title) };
        self.metadata.updated_at = Utc::now();
        self.save_metadata()
    }

    pub fn auto_title_from_user_task(&mut self, task: &str) -> Result<()> {
        if self
            .metadata
            .title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            return Ok(());
        }
        let Some(title) = derive_session_title(task) else {
            return Ok(());
        };
        self.metadata.title = Some(title);
        self.metadata.updated_at = Utc::now();
        self.save_metadata()
    }

    pub fn set_provider_model(
        &mut self,
        provider: impl Into<String>,
        model: Option<String>,
    ) -> Result<()> {
        self.metadata.provider = provider.into();
        self.metadata.model = model;
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
        )?;
        self.touch_metadata()
    }

    pub fn load_messages(&self) -> Result<Vec<SessionMessage>> {
        self.read_jsonl_if_exists("messages.jsonl")
    }

    pub fn load_recent_messages(&self, limit: usize) -> Result<Vec<SessionMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let messages = self.load_messages()?;
        let skip = messages.len().saturating_sub(limit);
        Ok(messages.into_iter().skip(skip).collect())
    }

    pub fn load_tool_calls(&self) -> Result<Vec<ToolCallRecord>> {
        self.read_jsonl_if_exists("tools.jsonl")
    }

    pub fn load_recent_tool_calls(&self, limit: usize) -> Result<Vec<ToolCallRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let records = self.load_tool_calls()?;
        let skip = records.len().saturating_sub(limit);
        Ok(records.into_iter().skip(skip).collect())
    }

    pub fn append_tool_call(&self, record: &ToolCallRecord) -> Result<()> {
        self.append_jsonl("tools.jsonl", record)?;
        self.append_audit_event("tool_call", serde_json::to_value(record)?)?;
        self.touch_metadata()
    }

    pub fn append_provider_transcript(&self, record: &ProviderTranscriptRecord) -> Result<()> {
        self.append_jsonl("provider_transcript.jsonl", record)?;
        self.touch_metadata()
    }

    pub fn load_provider_transcript(&self) -> Result<Vec<ProviderTranscriptRecord>> {
        self.read_jsonl_if_exists("provider_transcript.jsonl")
    }

    pub fn load_recent_provider_transcript(
        &self,
        limit: usize,
    ) -> Result<Vec<ProviderTranscriptRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let records = self.load_provider_transcript()?;
        let skip = records.len().saturating_sub(limit);
        Ok(records.into_iter().skip(skip).collect())
    }

    pub fn append_compact_boundary(&self, record: &CompactBoundaryRecord) -> Result<()> {
        self.append_jsonl("compact_boundaries.jsonl", record)?;
        self.append_audit_event(
            "compact_boundary",
            serde_json::json!({
                "id": record.id,
                "reason": record.reason,
                "omitted_group_count": record.omitted_group_count,
                "message_count_before": record.message_count_before,
                "message_count_after": record.message_count_after,
                "retained_segment_count": record.retained_segment.len(),
            }),
        )?;
        self.touch_metadata()
    }

    pub fn load_compact_boundaries(&self) -> Result<Vec<CompactBoundaryRecord>> {
        self.read_jsonl_if_exists("compact_boundaries.jsonl")
    }

    pub fn load_latest_compact_boundary(&self) -> Result<Option<CompactBoundaryRecord>> {
        Ok(self.load_compact_boundaries()?.into_iter().last())
    }

    pub fn append_file_history_snapshot(&self, record: &FileHistorySnapshot) -> Result<()> {
        self.append_jsonl("file_history.jsonl", record)?;
        self.touch_metadata()
    }

    pub fn load_file_history_snapshots(&self) -> Result<Vec<FileHistorySnapshot>> {
        self.read_jsonl_if_exists("file_history.jsonl")
    }

    pub fn load_recent_file_history_snapshots(
        &self,
        limit: usize,
    ) -> Result<Vec<FileHistorySnapshot>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let records = self.load_file_history_snapshots()?;
        let skip = records.len().saturating_sub(limit);
        Ok(records.into_iter().skip(skip).collect())
    }

    pub fn append_test_run(&self, record: &TestRunRecord) -> Result<()> {
        self.append_jsonl("tests.jsonl", record)?;
        self.append_audit_event("test_run", serde_json::to_value(record)?)?;
        self.touch_metadata()
    }

    pub fn load_test_runs(&self) -> Result<Vec<TestRunRecord>> {
        self.read_jsonl_if_exists("tests.jsonl")
    }

    pub fn load_recent_test_runs(&self, limit: usize) -> Result<Vec<TestRunRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let records = self.load_test_runs()?;
        let skip = records.len().saturating_sub(limit);
        Ok(records.into_iter().skip(skip).collect())
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

    pub fn load_audit_events(&self) -> Result<Vec<AuditEvent>> {
        let path = self
            .metadata
            .workspace
            .join(".deepcli")
            .join("logs")
            .join("audit.jsonl");
        Ok(read_audit_jsonl_path_if_exists(&path)?
            .into_iter()
            .filter(|event: &AuditEvent| event.session_id == self.metadata.id)
            .collect())
    }

    pub fn save_plan(&self, plan: &Plan) -> Result<()> {
        self.write_json("plan.json", plan)?;
        self.touch_metadata()
    }

    pub fn load_plan(&self) -> Result<Option<Plan>> {
        self.read_json_if_exists("plan.json")
    }

    pub fn write_plan_document(&self, document: &str) -> Result<()> {
        fs::write(self.path.join("plan.md"), document)
            .with_context(|| format!("failed to write plan document for {}", self.metadata.id))?;
        self.touch_metadata()
    }

    pub fn load_plan_document(&self) -> Result<Option<String>> {
        let path = self.path.join("plan.md");
        if !path.exists() {
            return Ok(None);
        }
        fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .map(Some)
    }

    pub fn save_goal(&self, goal: &GoalContract) -> Result<()> {
        self.write_json("goal.json", goal)?;
        self.touch_metadata()
    }

    pub fn load_goal(&self) -> Result<Option<GoalContract>> {
        self.read_json_if_exists("goal.json")
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
        self.enqueue_side_question_with_options(question, Vec::new())
    }

    pub fn enqueue_side_question_with_options(
        &self,
        question: impl Into<String>,
        options: Vec<String>,
    ) -> Result<SideQuestion> {
        let question = question.into();
        let options = options
            .into_iter()
            .map(|option| option.trim().to_string())
            .filter(|option| !option.is_empty())
            .take(8)
            .collect();
        let now = Utc::now();
        let item = SideQuestion {
            id: Uuid::new_v4(),
            question,
            options,
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
        let safe_name = sanitize_session_file_name(name);
        let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let mut path = diffs.join(format!("{stamp}-{safe_name}.diff"));
        let mut collision = 1usize;
        while path.exists() {
            path = diffs.join(format!("{stamp}-{collision}-{safe_name}.diff"));
            collision += 1;
        }
        fs::write(&path, diff)?;
        self.touch_metadata()?;
        Ok(path)
    }

    pub fn load_diffs(&self) -> Result<Vec<SessionDiffRecord>> {
        let diffs = self.path.join("diffs");
        if !diffs.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&diffs)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("diff") {
                continue;
            }
            let metadata = entry.metadata()?;
            let modified_at = metadata
                .modified()
                .map(DateTime::<Utc>::from)
                .unwrap_or_else(|_| DateTime::<Utc>::from(SystemTime::UNIX_EPOCH));
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            records.push(SessionDiffRecord {
                name,
                path,
                content: fs::read_to_string(entry.path())?,
                modified_at,
            });
        }
        records.sort_by_key(|record| record.modified_at);
        Ok(records)
    }

    pub fn load_recent_diffs(&self, limit: usize) -> Result<Vec<SessionDiffRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let records = self.load_diffs()?;
        let skip = records.len().saturating_sub(limit);
        Ok(records.into_iter().skip(skip).collect())
    }

    pub fn save_backup(&self, name: &str, content: &str) -> Result<PathBuf> {
        let backups = self.path.join("backups");
        fs::create_dir_all(&backups)?;
        let safe_name = sanitize_session_file_name(name);
        let now = Utc::now();
        let stamp = now.format("%Y%m%dT%H%M%S%.3fZ");
        let mut path = backups.join(format!("{stamp}-{safe_name}.bak"));
        let mut collision = 1usize;
        while path.exists() {
            path = backups.join(format!("{stamp}-{collision}-{safe_name}.bak"));
            collision += 1;
        }
        fs::write(&path, content)?;
        let backup_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        self.append_jsonl(
            "backups.jsonl",
            &SessionBackupIndexRecord {
                name: backup_name,
                path: path.clone(),
                target_path: PathBuf::from(name),
                created_at: now,
            },
        )?;
        self.touch_metadata()?;
        Ok(path)
    }

    pub fn load_backups(&self) -> Result<Vec<SessionBackupRecord>> {
        let backups = self.path.join("backups");
        if !backups.exists() {
            return Ok(Vec::new());
        }
        let index = self.read_jsonl_if_exists::<SessionBackupIndexRecord>("backups.jsonl")?;
        let mut records = Vec::new();
        for entry in fs::read_dir(&backups)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("bak") {
                continue;
            }
            let metadata = entry.metadata()?;
            let modified_at = metadata
                .modified()
                .map(DateTime::<Utc>::from)
                .unwrap_or_else(|_| DateTime::<Utc>::from(SystemTime::UNIX_EPOCH));
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            let indexed = index
                .iter()
                .rev()
                .find(|record| backup_index_matches(record, &path));
            records.push(SessionBackupRecord {
                name: indexed.map(|record| record.name.clone()).unwrap_or(name),
                path,
                target_path: indexed.map(|record| record.target_path.clone()),
                content: fs::read_to_string(entry.path())?,
                modified_at,
            });
        }
        records.sort_by_key(|record| record.modified_at);
        Ok(records)
    }

    pub fn load_recent_backups(&self, limit: usize) -> Result<Vec<SessionBackupRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let records = self.load_backups()?;
        let skip = records.len().saturating_sub(limit);
        Ok(records.into_iter().skip(skip).collect())
    }

    pub fn write_summary(&self, summary: &str) -> Result<()> {
        fs::write(self.path.join("summary.md"), summary)
            .with_context(|| format!("failed to write summary for {}", self.metadata.id))?;
        self.touch_metadata()
    }

    pub fn load_summary(&self) -> Result<Option<String>> {
        let path = self.path.join("summary.md");
        if !path.exists() {
            return Ok(None);
        }
        fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .map(Some)
    }

    fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.path.join("diffs"))
            .with_context(|| format!("failed to create {}", self.path.display()))
    }

    fn save_metadata(&self) -> Result<()> {
        self.write_json("metadata.json", &self.metadata)
    }

    fn touch_metadata(&self) -> Result<()> {
        let path = self.path.join("metadata.json");
        if !path.exists() {
            return Ok(());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut metadata: SessionMetadata = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        metadata.updated_at = Utc::now();
        fs::write(&path, serde_json::to_vec_pretty(&metadata)?)
            .with_context(|| format!("failed to write {}", path.display()))
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

    fn read_jsonl_if_exists<T: DeserializeOwned>(&self, name: &str) -> Result<Vec<T>> {
        let path = self.path.join(name);
        read_jsonl_path_if_exists(&path)
    }

    fn append_jsonl<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        self.ensure_dirs()?;
        let path = self.path.join(name);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        write_jsonl_line(&mut file, value)?;
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
        write_jsonl_line(&mut file, value)?;
        Ok(())
    }

    fn save_side_questions(&self, items: &[SideQuestion]) -> Result<()> {
        self.write_json("side_questions.json", &items)?;
        self.touch_metadata()
    }

    fn save_approval_requests(&self, items: &[ApprovalRequest]) -> Result<()> {
        self.write_json("approvals.json", &items)?;
        self.touch_metadata()
    }
}

fn read_jsonl_path_if_exists<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

fn read_audit_jsonl_path_if_exists(path: &Path) -> Result<Vec<AuditEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<AuditEvent>(line).ok())
        .collect())
}

fn write_jsonl_line<T: Serialize>(file: &mut fs::File, value: &T) -> Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    file.write_all(&line)?;
    Ok(())
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

fn sanitize_session_file_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.trim_matches('_').is_empty() {
        "diff".to_string()
    } else {
        sanitized
    }
}

fn derive_session_title(task: &str) -> Option<String> {
    let redacted = redact_sensitive_text(task);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = normalized.trim();
    if title.is_empty() || title.starts_with('/') {
        return None;
    }
    Some(truncate_title(title, 72))
}

fn truncate_title(title: &str, limit: usize) -> String {
    let char_count = title.chars().count();
    if char_count <= limit {
        return title.to_string();
    }
    let keep = limit.saturating_sub(3);
    let mut output = title.chars().take(keep).collect::<String>();
    output.push_str("...");
    output
}

fn backup_index_matches(record: &SessionBackupIndexRecord, path: &Path) -> bool {
    record.path == path
        || record.path.file_name().is_some_and(|record_name| {
            path.file_name()
                .is_some_and(|path_name| path_name == record_name)
        })
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
            .append_test_run(&TestRunRecord {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                passed: true,
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
        let messages = loaded.load_messages().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(loaded.load_recent_messages(1).unwrap(), messages);
        loaded.write_summary("done").unwrap();
        assert_eq!(loaded.load_summary().unwrap().as_deref(), Some("done"));
        loaded
            .write_plan_document("# Plan\n\n### Critical Files for Implementation\n- src/lib.rs\n")
            .unwrap();
        assert_eq!(
            loaded.load_plan_document().unwrap().as_deref(),
            Some("# Plan\n\n### Critical Files for Implementation\n- src/lib.rs\n")
        );
        assert_eq!(loaded.load_tool_calls().unwrap().len(), 1);
        assert_eq!(
            loaded.load_recent_tool_calls(1).unwrap()[0].tool,
            "read_file"
        );
        assert_eq!(loaded.load_test_runs().unwrap().len(), 1);
        assert_eq!(
            loaded.load_recent_test_runs(1).unwrap()[0].command,
            "cargo test"
        );
        assert!(loaded
            .load_audit_events()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "tool_call"));
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
    fn stores_recovery_context_records() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let now = Utc::now();
        let tool_call = ProviderTranscriptToolCall {
            id: "call_read".to_string(),
            call_type: "function".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        };
        let assistant = ProviderTranscriptRecord {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            name: None,
            tool_call_id: None,
            tool_calls: vec![tool_call],
            synthetic: false,
            created_at: now,
        };
        let tool = ProviderTranscriptRecord {
            role: "tool".to_string(),
            content: Some("file content".to_string()),
            reasoning_content: None,
            name: Some("read_file".to_string()),
            tool_call_id: Some("call_read".to_string()),
            tool_calls: Vec::new(),
            synthetic: false,
            created_at: now,
        };
        session.append_provider_transcript(&assistant).unwrap();
        session.append_provider_transcript(&tool).unwrap();
        session
            .append_compact_boundary(&CompactBoundaryRecord {
                id: Uuid::new_v4(),
                reason: "full_compact".to_string(),
                summary: "kept the current bug and latest test failure".to_string(),
                omitted_group_count: 3,
                message_count_before: 12,
                message_count_after: 6,
                retained_segment: vec![assistant.clone(), tool.clone()],
                created_at: now,
            })
            .unwrap();
        session
            .append_file_history_snapshot(&FileHistorySnapshot {
                tool: "read_file".to_string(),
                target: "src/lib.rs".to_string(),
                summary: "loaded parser entrypoint".to_string(),
                data: serde_json::json!({"path": "src/lib.rs"}),
                created_at: now,
            })
            .unwrap();

        let transcript = session.load_provider_transcript().unwrap();
        assert_eq!(transcript, vec![assistant, tool]);
        let boundary = session.load_latest_compact_boundary().unwrap().unwrap();
        assert_eq!(boundary.omitted_group_count, 3);
        assert_eq!(boundary.retained_segment.len(), 2);
        let snapshots = session.load_recent_file_history_snapshots(5).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].tool, "read_file");
        assert_eq!(snapshots[0].target, "src/lib.rs");
    }

    #[test]
    fn load_audit_events_skips_corrupt_workspace_lines() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        let other = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        let audit_path = dir.path().join(".deepcli/logs/audit.jsonl");
        let first = AuditEvent {
            session_id: session.id(),
            event_type: "provider_turn_started".to_string(),
            payload: serde_json::json!({"iteration": 1}),
            created_at: Utc::now(),
        };
        let corrupt = "{{not json";
        let unrelated = AuditEvent {
            session_id: other.id(),
            event_type: "tool_started".to_string(),
            payload: serde_json::json!({"tool": "read_file"}),
            created_at: Utc::now(),
        };
        let second = AuditEvent {
            session_id: session.id(),
            event_type: "provider_turn_completed".to_string(),
            payload: serde_json::json!({"elapsed_ms": 12}),
            created_at: Utc::now(),
        };
        let contents = format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::to_string(&first).unwrap(),
            corrupt,
            serde_json::to_string(&unrelated).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        fs::create_dir_all(audit_path.parent().unwrap()).unwrap();
        fs::write(&audit_path, contents).unwrap();

        let events = session.load_audit_events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "provider_turn_started");
        assert_eq!(events[1].event_type, "provider_turn_completed");
    }

    #[test]
    fn renames_session_and_reads_legacy_metadata_without_title() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();
        session.rename("compiler fix").unwrap();
        let loaded = store.load(&session.id().to_string()).unwrap();
        assert_eq!(loaded.metadata.title.as_deref(), Some("compiler fix"));

        let legacy = serde_json::json!({
            "id": uuid::Uuid::new_v4(),
            "workspace": dir.path(),
            "state": "new",
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "provider": "deepseek",
            "model": "deepseek-v4-pro"
        });
        let metadata: SessionMetadata = serde_json::from_value(legacy).unwrap();
        assert_eq!(metadata.title, None);
    }

    #[test]
    fn auto_titles_session_from_first_meaningful_task() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let mut session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        session
            .auto_title_from_user_task("  修复 compiler 项目\n并运行测试  ")
            .unwrap();
        let loaded = store.load(&session.id().to_string()).unwrap();
        assert_eq!(
            loaded.metadata.title.as_deref(),
            Some("修复 compiler 项目 并运行测试")
        );

        session.auto_title_from_user_task("改成别的标题").unwrap();
        let loaded = store.load(&session.id().to_string()).unwrap();
        assert_eq!(
            loaded.metadata.title.as_deref(),
            Some("修复 compiler 项目 并运行测试")
        );
    }

    #[test]
    fn auto_title_skips_commands_redacts_secrets_and_truncates_long_tasks() {
        assert_eq!(derive_session_title("/status"), None);
        assert_eq!(
            derive_session_title("api_key = abc123"),
            Some("api_key = <redacted>".to_string())
        );

        let long_title = derive_session_title(
            "please inspect the compiler project, fix all failing parser tests, then run the full validation suite",
        )
        .unwrap();
        assert_eq!(long_title.chars().count(), 72);
        assert!(long_title.ends_with("..."));
    }

    #[test]
    fn loads_sessions_by_unique_prefix_and_reports_ambiguous_prefixes() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let now = Utc::now();
        let first_id = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let second_id = Uuid::parse_str("11111112-1111-4111-8111-111111111111").unwrap();
        for id in [first_id, second_id] {
            let metadata = SessionMetadata {
                id,
                title: None,
                workspace: dir.path().to_path_buf(),
                state: SessionState::New,
                created_at: now,
                updated_at: now,
                provider: "deepseek".to_string(),
                model: Some("deepseek-v4-pro".to_string()),
            };
            let path = store.root.join(id.to_string());
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("metadata.json"),
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .unwrap();
        }

        assert_eq!(store.resolve_id("11111111").unwrap(), first_id.to_string());
        assert_eq!(store.load("11111111").unwrap().id(), first_id);
        let ambiguous = store.resolve_id("1111111").unwrap_err().to_string();
        assert!(ambiguous.contains("ambiguous"));
        assert!(store
            .resolve_id("missing")
            .unwrap_err()
            .to_string()
            .contains("not found"));
        assert!(store
            .resolve_id("../outside")
            .unwrap_err()
            .to_string()
            .contains("unique UUID prefix"));
    }

    #[test]
    fn session_list_sorts_by_latest_recorded_activity() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let first = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = store
            .create(
                dir.path(),
                "deepseek".to_string(),
                Some("deepseek-v4-pro".to_string()),
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        first.append_message("user", "继续修复 compiler").unwrap();

        let sessions = store.list().unwrap();
        assert_eq!(sessions[0].id, first.id());
        assert_eq!(sessions[1].id, second.id());
        assert!(sessions[0].updated_at > sessions[1].updated_at);
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
    fn stores_side_question_options() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        let item = session
            .enqueue_side_question_with_options(
                "Which route should the plan use?",
                vec!["Validate first".to_string(), "Implement Task 6".to_string()],
            )
            .unwrap();

        assert_eq!(
            item.options,
            vec!["Validate first".to_string(), "Implement Task 6".to_string()]
        );
        let loaded = session.load_side_questions().unwrap();
        assert_eq!(loaded[0].options, item.options);
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

    #[test]
    fn stores_diff_history_without_overwriting_same_target() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        let first = session.save_diff("src/lib.rs", "-old\n+first\n").unwrap();
        let second = session
            .save_diff("src/lib.rs", "-first\n+second\n")
            .unwrap();

        assert_ne!(first, second);
        let diffs = session.load_diffs().unwrap();
        assert_eq!(diffs.len(), 2);
        assert!(diffs[0].name.contains("src_lib.rs"));
        assert!(diffs[0].content.contains("+first"));
        assert!(diffs[1].content.contains("+second"));
        assert_eq!(session.activity_summary().unwrap().diff_count, 2);
        assert_eq!(
            session.load_recent_diffs(1).unwrap()[0].content,
            diffs[1].content
        );
    }

    #[test]
    fn stores_backup_history_without_overwriting_same_target() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let session = store
            .create(dir.path(), "deepseek".to_string(), None)
            .unwrap();

        let first = session.save_backup("src/lib.rs", "first backup").unwrap();
        let second = session.save_backup("src/lib.rs", "second backup").unwrap();

        assert_ne!(first, second);
        let backups = session.load_backups().unwrap();
        assert_eq!(backups.len(), 2);
        assert!(backups[0].name.contains("src_lib.rs"));
        assert_eq!(backups[0].target_path, Some(PathBuf::from("src/lib.rs")));
        assert_eq!(backups[0].content, "first backup");
        assert_eq!(backups[1].content, "second backup");
        assert_eq!(session.activity_summary().unwrap().backup_count, 2);
        assert_eq!(
            session.load_recent_backups(1).unwrap()[0].content,
            backups[1].content
        );
    }
}
