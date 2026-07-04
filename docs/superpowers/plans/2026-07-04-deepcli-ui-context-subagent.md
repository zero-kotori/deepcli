# Deepcli UI Context Subagent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement UI dialogs, extract context preparation into a context manager, and make sub-agents runnable, recoverable, and observable.

**Architecture:** Add a `src/ui/dialogs.rs` owner for terminal overlays, a `src/context_manager.rs` owner for provider context preparation, and persistent lifecycle/event support around `src/agents.rs` plus `/agent` commands. Keep existing runtime/session/config/tool paths authoritative and avoid demo-only behavior.

**Tech Stack:** Rust 2021, ratatui, crossterm, tokio, serde JSON/JSONL, existing deepcli session/config/tool/provider modules.

---

### Task 1: UI Dialog Owner

**Files:**
- Create: `src/ui/dialogs.rs`
- Modify: `src/ui.rs`
- Modify: `src/ui/chat_view.rs`
- Test: `src/ui/tests.rs`
- Docs: `docs/MODULES/ui.md`

- [ ] **Step 1: Write failing dialog shell tests**

Add tests proving `TuiState` can open and close a dialog, `Esc` closes it before exiting the TUI, and rendering replaces the input area with the dialog body.

Run: `cargo test ui_dialog --lib`

Expected: tests fail because `TuiDialog` and dialog routing do not exist.

- [ ] **Step 2: Implement minimal dialog state and renderer**

Create `TuiDialog`, `DialogKind`, `DialogAction`, `render_dialog`, and `handle_dialog_key`. Wire `TuiState.dialog`, key routing, mouse fallthrough prevention, and chat-view rendering.

Run: `cargo test ui_dialog --lib`

Expected: dialog shell tests pass.

- [ ] **Step 3: Add UI module ownership guard**

Extend `docs/MODULES/ui.md` and add/adjust the contract test that documents `src/ui/dialogs.rs` as the dialog owner.

Run: `cargo test ui_module_docs --test mvp_contract`

Expected: owner docs test passes.

### Task 2: Permission And Interview Dialogs

**Files:**
- Modify: `src/ui/dialogs.rs`
- Modify: `src/ui/approvals.rs`
- Modify: `src/ui.rs`
- Test: `src/ui/tests.rs`

- [ ] **Step 1: Write failing approval dialog tests**

Add tests for pending approval rendering, selection, approve on `Enter`, deny on `d`, and active-session operation while runtime is absent.

Run: `cargo test approval_dialog --lib`

Expected: tests fail because approvals still use the inline prompt.

- [ ] **Step 2: Implement approval dialog**

Move approval prompt view/action routing into dialog APIs while keeping `SessionStore` and `SessionMonitor` as data sources.

Run: `cargo test approval_dialog --lib`

Expected: approval dialog tests pass and existing approvals tests continue to pass.

- [ ] **Step 3: Write failing interview dialog tests**

Add tests for opening a side question as a dialog, multiline answer editing, save on `Enter`, and cancel on `Esc`.

Run: `cargo test interview_dialog --lib`

Expected: tests fail because `side_question_prompt` is still a separate prompt state.

- [ ] **Step 4: Implement interview dialog**

Replace `side_question_prompt` rendering and key routing with the dialog owner while preserving current side-question persistence semantics.

Run: `cargo test interview_dialog approval_dialog --lib`

Expected: interview and approval dialog tests pass.

### Task 3: Diff, Agent Editor, And Settings Dialogs

**Files:**
- Modify: `src/ui/dialogs.rs`
- Modify: `src/ui/monitor_changes.rs`
- Modify: `src/ui/monitor_library.rs`
- Modify: `src/ui/input_submission.rs`
- Modify: `src/agents.rs`
- Modify: `src/config.rs`
- Test: `src/ui/tests.rs`
- Test: `src/agents.rs`
- Test: `src/config.rs`

- [ ] **Step 1: Write failing diff dialog tests**

Add tests for opening selected patch from Changes, scrolling patch content, switching selection with `[` and `]`, and refusing to run git from render.

Run: `cargo test diff_dialog --lib`

Expected: tests fail because selected patches only render inside the Changes tab.

- [ ] **Step 2: Implement diff dialog**

Expose selected diff section helpers from `monitor_changes`, render patch details in a dialog, and reuse existing scroll state.

Run: `cargo test diff_dialog --lib`

Expected: diff dialog tests pass.

- [ ] **Step 3: Write failing agent editor tests**

Add tests for editing queued task text/scope/tool hints/context, saving through `AgentStore`, and rejecting edits for non-queued tasks.

Run: `cargo test agent_editor --lib`

Expected: tests fail because no editor exists and `AgentStore` lacks update helpers.

- [ ] **Step 4: Implement agent editor**

Add `AgentStore::update_subagent_task` with validation, then add the dialog fields and save/cancel behavior.

Run: `cargo test agent_editor --lib`

Expected: agent editor tests pass.

- [ ] **Step 5: Write failing settings dialog tests**

Add tests for displaying whitelisted config fields, updating numeric values, rejecting invalid values, and never exposing credential values.

Run: `cargo test settings_dialog --lib`

Expected: tests fail because settings are only command-driven.

- [ ] **Step 6: Implement settings dialog**

Add whitelisted settings model and save through existing config update/validation helpers.

Run: `cargo test settings_dialog --lib`

Expected: settings dialog tests pass.

### Task 4: Context Manager Extraction

**Files:**
- Create: `src/context_manager.rs`
- Modify: `src/lib.rs`
- Modify: `src/runtime.rs`
- Test: `src/context_manager.rs`
- Test: `src/runtime.rs`
- Docs: `docs/MODULES/runtime.md` if present, otherwise `docs/ARCHITECTURE.md`

- [ ] **Step 1: Write failing owner/delegation tests**

Add tests proving context preparation is available through `ContextManager` and runtime no longer owns compaction-only helpers as private runtime logic.

Run: `cargo test context_manager --lib`

Expected: tests fail because the module does not exist.

- [ ] **Step 2: Move compaction types and helpers**

Move `ContextCompactionOptions`, `ContextPreparation`, microcompact, full compact, tail compact, retry compact, token estimation, and retained segment helpers into `src/context_manager.rs`.

Run: `cargo test context_manager --lib`

Expected: moved compaction tests pass.

- [ ] **Step 3: Wire runtime delegation**

Replace runtime calls with `ContextManager::from_config(&self.config).prepare(...)` and keep session boundary persistence in runtime.

Run: `cargo test prepare_messages context_retry output_limit --lib`

Expected: existing runtime behavior tests pass.

### Task 5: Agent Lifecycle Store And Events

**Files:**
- Modify: `src/agents.rs`
- Test: `src/agents.rs`

- [ ] **Step 1: Write failing lifecycle tests**

Add tests for started/completed/failed transitions, event append/list, stale heartbeat detection, and backward-compatible loading of old queued task JSON.

Run: `cargo test agents:: --lib`

Expected: tests fail because lifecycle fields and events do not exist.

- [ ] **Step 2: Implement lifecycle persistence**

Add optional lifecycle fields with serde defaults, transition helpers, event JSONL helpers, and stale-running detection.

Run: `cargo test agents:: --lib`

Expected: lifecycle tests pass.

### Task 6: Agent Commands, Runner, And Tool Integration

**Files:**
- Modify: `src/commands/agent.rs`
- Modify: `src/commands/help.rs`
- Modify: `src/commands/tests.rs`
- Modify: `src/tools.rs`
- Modify: `src/tools/declarations.rs`
- Modify: `src/tools/schema.rs`
- Modify: `src/tools/validation.rs`
- Create: `src/subagent_runner.rs` or extend `src/agents.rs` if still small enough
- Modify: `src/lib.rs`
- Test: `src/commands/tests.rs`
- Test: `src/tools.rs`

- [ ] **Step 1: Write failing command tests**

Add command tests for `agent spawn --no-start`, `agent resume`, `agent logs`, lifecycle JSON fields in `show/list`, and nextActions for queued/running/stale/completed tasks.

Run: `cargo test agent_ --lib`

Expected: tests fail because command actions are unsupported.

- [ ] **Step 2: Implement command parsing and JSON output**

Extend `/agent` parsing and projection around lifecycle fields without starting real runtime yet.

Run: `cargo test agent_ --lib`

Expected: read/projection command tests pass.

- [ ] **Step 3: Write failing runner/tool tests**

Add tests proving `spawn_subagent` starts or schedules a runnable task, writes observable lifecycle events, and returns metadata including log path and child session id when available.

Run: `cargo test spawn_subagent subagent_runner --lib`

Expected: tests fail because no runner exists.

- [ ] **Step 4: Implement minimal real runner**

Create the runner that builds a child `AgentRuntime`, marks lifecycle transitions, writes heartbeat/events, and exposes foreground `resume`. `spawn_subagent` starts via the runner when safe and reports failure through structured output if start fails.

Run: `cargo test spawn_subagent subagent_runner agent_ --lib`

Expected: runner, tool, and command tests pass.

### Task 7: Final Verification And Commit

**Files:**
- Review all touched files with `git diff --stat` and `git diff`.

- [ ] **Step 1: Format**

Run: `cargo fmt`

Expected: command succeeds.

- [ ] **Step 2: Focused tests**

Run the focused commands from Tasks 1-6 again.

Expected: all focused tests pass.

- [ ] **Step 3: Product gates**

Run:

```bash
./scripts/deepcli scorecard --json
./scripts/deepcli privacy --json --fail-on-findings --no-history
```

Expected: scorecard succeeds and privacy status is `ok` with no actionable findings.

- [ ] **Step 4: Commit implementation**

Review `git status --short`, ensure no local artifacts or sensitive files are staged, then commit with:

```bash
git add src docs Cargo.toml Cargo.lock
git commit -m "feat: add ui dialogs context manager and subagents"
```
