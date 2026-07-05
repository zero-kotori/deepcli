# deepcli Agent Context

This repository is the active workspace for deepcli. Prefer the current worktree and the authoritative docs in this repository over prior chat memory.

## Project Goal

Maintain deepcli as a local-first AI coding CLI with real, testable product behavior:

1. Keep the native terminal, provider/runtime, tool, permission, session, verification, and documentation paths coherent.
2. Use the current codebase and docs as the source of truth before editing.
3. Implement narrowly scoped improvements that connect to the real runtime/provider/tool/session path.
4. Verify changes with focused tests and local checks before committing.

Old product-loop wording is historical context, not a standing product requirement. The `recipes sota` command/topic remains a compatibility name for the existing product-loop recipe; do not treat that name as permission to add broad benchmark or aspirational requirements.

## Current Operating Rules

- Default user-facing language is Chinese.
- Keep changes scoped to the current product gap.
- Never ship demo-only, simulated, or fake implementations as product behavior; fake fixtures belong only in tests or harnesses.
- Do not leave local benchmark/export artifacts in the worktree.
- Keep credentials, logs, sessions, and generated local evidence out of commits.
- Expected Git commit identity: `zero-kotori <kotorizero8@gmail.com>`.
- Before committing, run relevant tests plus privacy/name scans described in `docs/ai/CONTEXT.md`.
- After each completed task round, sync the local work by creating a Git commit once verification and privacy checks pass, unless the user explicitly asks not to commit or a blocker prevents a safe commit.

## Product Context

- Product documentation lives in `README.md`, `docs/FEATURES.md`, `docs/CORE_FEATURES.md`, `docs/COMMANDS.md`, `docs/ARCHITECTURE.md`, `docs/HARNESS.md`, `docs/MODULES/`, `docs/ai/REQUIREMENTS.md`, and `docs/ai/TECHNICAL_PLAN.md`.
- Current handoff context lives in `docs/ai/CONTEXT.md`.
- Local health/evidence commands include `deepcli scorecard --json`, `deepcli round --json`, `deepcli benchmark status --json`, and `deepcli preflight --json`.
- Benchmark evidence is intentionally local under `.deepcli/benchmarks/` and should not be committed.
