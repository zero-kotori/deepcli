# Deepcli UI Context Subagent Design

## Goal

Deepcli will ship three connected improvements in order:

1. Add real TUI components for permission approvals, diff inspection, agent editing, settings, and interview questions.
2. Move provider-context preparation and compaction out of `src/runtime.rs` into an independent context manager.
3. Upgrade `spawn_subagent` from a persisted descriptor into a runnable, recoverable, observable background sub-agent loop.

Each part must be backed by the existing runtime, session, config, diff, permission, and agent stores. No product behavior may rely on demo data or simulated runtime state.

## Scope

This design covers the local-first terminal product. The UI components are ratatui/crossterm overlays inside the current native TUI, not a web UI and not a separate frontend stack. The background sub-agent loop runs from the same deepcli binary and persists its lifecycle in `.deepcli/agents`, so one-shot commands, the TUI, and future external UIs can observe the same state.

The implementation order is fixed:

1. UI overlays and dialogs.
2. Context manager extraction.
3. Runnable background sub-agents.

## Part 1: UI Dialogs

### Architecture

Add a focused dialog owner under `src/ui/dialogs.rs` and keep `src/ui.rs` as the event-loop orchestrator. `TuiState` gains one top-level `dialog: Option<TuiDialog>` field. Dialog rendering and keyboard handling route through this owner before the main input box, command palette, monitor tabs, and transcript scrolling.

Existing owners remain authoritative for their data:

- Permission dialog reads pending approvals from `SessionMonitor` and applies decisions through `SessionStore`.
- Diff dialog reads `WorkspaceChangesSnapshot` and selected `WorkspaceDiffSection` from the Changes owner.
- Agent editor reads and writes `AgentStore` task descriptors.
- Settings dialog reads and writes `AppConfig` via the existing `/config` command path or config update helpers.
- Interview dialog uses existing side-question records, answers through `SessionStore`, and replaces the current inline BTW prompt path.

### Dialog Behavior

All dialogs share the same shell:

- `Esc` closes the dialog unless the dialog is saving.
- `Tab` moves focus inside editable dialogs.
- `Enter` activates the primary action for selection dialogs or submits an editable field.
- `PageUp` and `PageDown` scroll long read-only content.
- Dialogs never execute high-risk actions on mouse hover or passive selection.

Permission dialog:

- Opens automatically when pending approvals exist, replacing the current input-area approval prompt.
- Shows selected approval or open question details.
- `Enter` approves an approval request or opens the interview dialog for a question.
- `d` denies approval requests.
- It must continue to work while an agent is running by loading the active persisted session when the in-memory runtime is not available.

Diff dialog:

- Opens from the Changes tab for the selected file patch.
- Shows the selected file path, truncation status, and patch body.
- Supports `[`, `]`, `PageUp`, and `PageDown` using the current Changes selection model.
- Does not execute `git` from the render path; it only reads the cached `WorkspaceChangesSnapshot`.

Agent editor:

- Opens from the Library tab or `/agent show` quick action context.
- Edits task text, read scope, write scope, allowed tools, and context hints for queued tasks.
- Refuses edits for running, completed, or failed tasks in the first implementation because those states need lifecycle audit preservation.
- Saves through `AgentStore`, reusing path normalization and allowed-tool validation.

Settings dialog:

- Starts with a conservative whitelist: provider turn timeout, max tool iterations, max context tokens, reserved output tokens, max sub-agent depth, permission mode, and default provider/model when already represented in `AppConfig`.
- Writes through existing config validation. Invalid values stay in the dialog and surface the validation error in the status line.
- Does not edit credentials or secret values.

Interview dialog:

- Replaces `side_question_prompt` as a first-class dialog.
- Shows the persisted question text and a multiline answer box.
- Saves answers to the current session using existing side-question APIs.
- Leaves no explanatory demo text in the product UI.

### Testing

Unit tests cover dialog selection, close behavior, editor validation, settings validation, and persistence. Rendering tests assert that the main input box is replaced by the active dialog and that text fits within the target area for narrow terminal widths.

## Part 2: Context Manager

### Architecture

Create `src/context_manager.rs` as the owner for provider-context preparation. It owns:

- `ContextCompactionOptions`
- `ContextPreparation`
- microcompaction of large tool outputs
- full provider-assisted compaction
- tail compaction fallback
- retry compaction helpers for prompt-too-long and max-output recovery
- retained-segment projection for `CompactBoundaryRecord`
- token estimation wrapper around `ProviderClient::count_tokens`

`AgentRuntime` keeps loop orchestration, progress events, session writes, and provider calls. It calls:

```rust
ContextManager::new(config).prepare(provider, messages, tools, timeout).await
```

The manager returns the prepared messages, token estimates, compaction flags, full-compaction errors, and optional `CompactBoundaryRecord`. Runtime remains responsible for appending the boundary to the active session.

### Data Flow

1. Runtime builds system/user/tool messages.
2. Runtime asks the context manager to prepare messages for the provider.
3. Context manager microcompacts old low-value tool outputs.
4. If estimated input tokens still exceed the threshold, it asks the configured provider for a full summary.
5. If still too large, it performs deterministic tail compaction.
6. Context manager returns the resulting message list plus observable compaction metadata.
7. Runtime records `provider_turn_started`, `context` metadata, and compact boundary persistence exactly as before.

### Compatibility

The first extraction must be behavior-preserving. Existing tests for microcompaction, full compaction, tail compaction, prompt-too-long recovery, and output-limit recovery move from runtime tests to context-manager tests or continue to call the public context-manager functions.

## Part 3: Background Sub-Agent Loop

### Architecture

Extend `src/agents.rs` into the persistent lifecycle owner and add a runner module, either `src/agents/runner.rs` or a sibling `src/subagent_runner.rs` if the file grows too large.

`SubagentTask` gains durable runtime fields:

- `status`: queued, running, completed, failed.
- `child_session_id`: session id created for the child agent.
- `started_at`, `completed_at`.
- `pid` when launched as a child process.
- `last_heartbeat_at`.
- `exit_code`.
- `summary`.
- `error`.
- `event_log_path`.

Add append-only JSONL events under `.deepcli/agents/events/<task-id>.jsonl`. Events include queued, started, heartbeat, output, completed, failed, recovered, and stale-running-detected.

### Command Surface

Extend `/agent` and `deepcli agent`:

- `agent spawn <task>` creates a task and starts it by default unless `--no-start` is passed.
- `agent resume <id>` runs or resumes a queued, running, or failed task using the same persisted lifecycle.
- `agent resume <id>` resumes a queued or stale-running task.
- `agent logs <id> [--json]` reads the JSONL event log.
- `agent show/list --json` include lifecycle fields, child session id, heartbeat age, log path, and next actions.

The `spawn_subagent` tool creates the task through `AgentStore` and starts it through the runner. The tool result returns structured data with task id, status, child session id if available, log path, and next actions.

### Runner Behavior

The minimal runnable loop uses a real `AgentRuntime` with the current workspace, provider config, permission config, and bounded sub-agent depth. It creates or resumes a child session, builds a task prompt from:

- the parent task text
- parent session id
- read/write scope
- allowed tool hints
- optional context hints

The runner writes lifecycle events before and after each durable state change. A heartbeat is updated while the child runtime is active. If a process dies without marking the task completed or failed, later `agent show`, `agent list`, or `agent resume` detects the stale heartbeat and marks the task recoverable rather than silently leaving it running forever.

### Recoverability

Recoverability means the task descriptor, child session id, status, and event log survive CLI exit. The first implementation does not need token-level continuation inside a provider turn. It must be able to:

- list a task after the original TUI exits;
- show whether it completed, failed, is running, or appears stale;
- resume a queued or stale task with the same descriptor;
- expose the child session id for normal `deepcli resume <id>` inspection.

### Observability

Observability is provided by:

- `/agent list --json`
- `/agent show <id> --json`
- `/agent logs <id> --json`
- Library tab summary
- Agent editor state labels
- session audit events emitted by the child runtime

The JSON schema remains `deepcli.agent.inspect.v1` unless a breaking output format requires a new schema. Additive lifecycle fields stay under the existing schema.

## Error Handling

Path scopes are normalized inside the workspace and reject parent traversal. Allowed tool names are validated against the current tool registry. Settings writes run config validation before persisting. Dialog saves report validation errors without losing the user's typed input.

Sub-agent runner failures mark the task failed with an error string and append a failed event. A missing event log does not make `agent show` fail; it reports the task and an empty event list with a diagnostic field for logs.

## Verification

Each part must be implemented test-first.

Part 1 verification:

- dialog state and keyboard unit tests;
- approval and interview persistence tests;
- diff dialog selection and scroll tests;
- agent editor queued-task save/reject tests;
- settings validation tests.

Part 2 verification:

- existing compaction tests still pass after extraction;
- new context-manager owner tests cover microcompact, full compact, tail compact, and retained boundary projection;
- runtime tests prove it delegates context preparation instead of owning compaction internals.

Part 3 verification:

- agent store lifecycle tests;
- command tests for `spawn`, `run`, `resume`, `show`, `list`, and `logs`;
- tool test proving `spawn_subagent` starts or schedules a runnable task and returns observable metadata;
- stale heartbeat detection test;
- runner test using the existing fake provider/runtime test patterns where possible.

Final verification:

- focused `cargo test` commands for changed modules;
- `./scripts/deepcli scorecard --json`;
- `./scripts/deepcli privacy --json --fail-on-findings --no-history`;
- `git status --short` review before commit.
