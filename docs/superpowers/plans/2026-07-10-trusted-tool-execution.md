# Trusted Tool Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every model-requested tool action pass through a host-owned, invocation-bound permission boundary and report its real execution outcome.

**Architecture:** Provider schemas carry operation data only. `ToolExecutor` derives permission context, requests or consumes a single-use session grant bound to a canonical argument digest, then projects a typed success outcome through runtime and persistence. Canonical workspace paths, ignore policy, test-command validation, and child-process environment scrubbing close the adjacent escape paths.

**Tech Stack:** Rust 2021, serde/serde_json, sha2, crossterm, tokio, existing unit and Cargo integration tests.

---

## File Map

- Create `src/tools/authorization.rs`: canonical invocation digest, redacted summary, shell-control detection, test-command validation, and server-derived shell traits.
- Modify `src/tools/schema.rs`: remove authorization and self-reported safety properties from provider schemas.
- Modify `src/tools/declarations.rs`: remove model-originated explicit approval from permission context.
- Modify `src/session.rs`: persist invocation-bound approval metadata, deduplicate requests, count confirmations, and consume grants once.
- Modify `src/permissions.rs`: remove explicit-approval bypass and limit deterministic auto-review to recognized safe test/build commands.
- Modify `src/tools/file.rs`: canonical/nearest-existing-ancestor workspace containment and patch-path validation.
- Modify `src/tools/process.rs`: scrub provider credentials and ensure cancellable child cleanup defaults.
- Modify `src/tools.rs`: use invocation grants, enforce ignore/test policy, and propagate typed tool success.
- Modify `src/runtime.rs`: remove the tool-less prompt path, pause immediately for approvals, and propagate tool outcomes.
- Modify `src/ui/native_terminal.rs`: render actionable invocation-bound pending approvals.
- Modify `src/commands/approval.rs` and command tests: expose confirmation/grant state without leaking raw arguments.
- Modify authoritative tool, permission, runtime, feature, and core-contract docs to match behavior.

### Task 1: Lock Down Provider Input

- [x] Add failing tests asserting every provider schema omits `approved`, `writes_files`, and `requires_network`, and that extra authorization fields fail validation.
- [x] Run the focused schema tests and confirm they fail for the current schemas.
- [x] Remove the untrusted properties and model-side boolean reads.
- [x] Run the focused schema tests and confirm they pass.

### Task 2: Add Invocation-Bound Grants

- [x] Add failing session tests for pending-request deduplication, exact digest matching, one-time consumption, legacy unbound rejection, and two-step double confirmation.
- [x] Run the session tests and confirm the missing fields/state transitions fail.
- [x] Add canonical invocation hashing and redacted summaries in `authorization.rs`.
- [x] Extend `ApprovalRequest` with backward-compatible serde defaults and implement request, approve, deny, clear, and consume transitions.
- [x] Update approval text/JSON projection and runtime approval updates.
- [x] Run session and approval command tests and confirm they pass.

### Task 3: Enforce Server-Derived Risk

- [x] Add failing permission/tool tests proving model self-approval, arbitrary `run_tests`, compound shell, network shell, and destructive commands cannot auto-authorize.
- [x] Run the focused tests and confirm each fails for the expected bypass.
- [x] Remove `explicit_approval` from tool permission requests.
- [x] Implement conservative shell analysis and a deterministic reviewer restricted to validated test/build invocations.
- [x] Restrict `run_tests` to discovered-command token prefixes without shell control operators.
- [x] Run permission and tool tests and confirm they pass.

### Task 4: Enforce Filesystem And Process Boundaries

- [x] Add failing tests for existing and new paths through an escaping symlink, direct `.env`/credential access, patch targets outside policy, and inherited provider credentials.
- [x] Run the focused tests and confirm the current lexical/env behavior fails.
- [x] Implement canonical/nearest-existing-ancestor resolution and `DeepIgnore` enforcement for provider file operations.
- [x] Validate every patch target before `git apply`.
- [x] Remove deepcli/provider secret variables from async, timeout, stdin, and blocking child commands; set kill-on-drop for async children.
- [x] Run filesystem and process tests and confirm they pass.

### Task 5: Propagate Real Tool Outcomes

- [x] Add failing tests showing non-zero shell, failed tests, and timeouts currently produce `ok: true`, `Succeeded`, and successful progress.
- [x] Run the focused outcome tests and confirm the false-success behavior.
- [x] Add success state to `ToolExecution` and use it for provider JSON, lifecycle records, runtime progress, and failure accounting.
- [x] Run tool/runtime tests and confirm they pass.

### Task 6: Unify Runtime Capability And Approval Pause

- [x] Add failing runtime tests for pending approval as a non-overridable blocker and for short prompts retaining the normal tool-capable route.
- [x] Run the focused runtime tests and confirm the fast path/blocker behavior fails.
- [x] Remove the tool-less `provider.stream` shortcut while retaining streamed events in the normal agent loop.
- [x] Stop the current turn immediately after an approval-bound tool result and persist `AwaitingApproval` without marking completion.
- [x] Render pending invocation summaries and confirmation progress in the native terminal.
- [x] Run runtime/UI tests and confirm they pass.

### Task 7: Synchronize Documentation And Verify

- [x] Update `docs/FEATURES.md`, `docs/CORE_FEATURES.md`, `docs/ARCHITECTURE.md`, `docs/MODULES/tools.md`, `docs/MODULES/permissions.md`, `docs/MODULES/runtime.md`, `docs/ai/REQUIREMENTS.md`, and `docs/ai/TECHNICAL_PLAN.md` only where the changed contracts are authoritative.
- [x] Run focused unit and contract tests.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo test --all-targets`.
- [x] Run `cargo clippy --all-targets -- -D warnings`.
- [x] Run `cargo build --all-targets` and `./scripts/native-terminal-smoke`.
- [x] Run `./scripts/deepcli preflight --quick --json` and `./scripts/deepcli privacy --no-history --json`.
- [x] Run `git diff --check`, inspect `git status`, and verify no local evidence or secrets are staged.
- [x] Verify the intended Git identity is `zero-kotori <kotorizero8@gmail.com>` and prepare the verified scope for commit.

### Additional Audit Closures

- [x] Pin public DNS addresses across redirects and bound Web response bodies.
- [x] Bind Git commit approval to the staged tree and update HEAD with compare-and-swap without repository hooks.
- [x] Enforce sub-agent allowed-tools, canonical read/write scopes, scoped workspace context, host-owned depth, and nested capability narrowing.
- [x] Restrict parallel batches to scope-safe local reads so approval-producing tools serialize.
- [x] Enable bracketed paste and sanitize untrusted terminal output, including split stream escape sequences.
- [x] Recursively merge default/global/project JSON configuration before environment overrides and keep `autoReviewer` disabled by default.
