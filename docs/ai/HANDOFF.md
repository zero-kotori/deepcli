# deepcli HARNESS Refactor Handoff

Updated: 2026-06-28 (after `/goal` and `/diagnose` command-handler splits)

## Current Stop Point

The current stop point is after the `/goal` and `/diagnose` command-handler extractions from `src/commands.rs`. The worktree is clean after the latest commits. The active long-term goal is still the HARNESS refactor described in `docs/ai/HARNESS_REFACTOR_PLAN.md`; do not treat this handoff as completion of that goal.

`src/commands.rs` is now ~34.7k lines (down from ~36.1k) and remains the largest complexity hotspot.

## Recent Commits

- `8de0aee refactor: split command diagnose handler`
- `29ddd35 refactor: split command goal handler`
- `60e0772 refactor: split command resume handler`
- `2c32aca refactor: split command git handler`
- `09c18d6 refactor: split command web handler`
- `3d54c40 refactor: split command prompt handler`

## What Was Completed

- Added `src/commands/goal.rs` for `/goal` show/start/clear/status/gate handling, default goal contract creation and guard-plan generation, goal readiness collection, goal session selection, and goal text/JSON formatting. `build_round_goal_status` in `src/commands.rs` keeps consuming the goal logic through crate-internal re-exports (`select_goal_session`, `collect_goal_readiness`, `GoalSessionSource`, `GoalPlanReadiness`, `GoalAcceptanceEvidence`), so `/round` goal-status summaries still share the same readiness contract.
- Added `src/commands/diagnose.rs` for `/diagnose` and `/support` handling: option parsing, diagnostics report JSON, redacted support-bundle generation and artifacts, issue templates, and diagnose next actions. It delegates workspace health to the `/doctor` handler and session diagnosis to the `/session` handler through `super::`. `workspace_relative_display` deliberately stayed in `src/commands.rs` because the benchmark code also uses it; `parse_diagnose_options` is re-exported `#[cfg(test)]`-only because only tests call it directly.
- Kept `docs/MODULES/commands.md` synchronized with each new command owner.
- Extended `tests/mvp_contract.rs::commands_module_docs_cover_split_source_files` so `src/commands/goal.rs` and `src/commands/diagnose.rs` must exist and be documented.

## Verification Method

Red-green flow per split:

- Red first: add the new `src/commands/<name>.rs` entry to `commands_module_docs_cover_split_source_files` and observe the expected failure `<file> should exist for command module ownership`.
- Green: move the code, add `mod <name>;` plus the `pub(crate) use` re-exports, update `docs/MODULES/commands.md`, then the contract test passes.

Regression proof on Windows: `cargo test commands::tests --lib` reports 23 pre-existing platform-dependent failures from tests that execute real POSIX shell / git / cargo (`verify_*`, `benchmark_*`, `git_status_*`, `doctor_shell_*`, `test_run_*`, `global_diagnose_bundle_*`, `completion_install_*`, `agent_list_*`, `skill_list_*`, `round_can_run_benchmark_suite_*`, `gate_without_current_session_*`). They fail identically on a pristine `git stash -u` baseline. Each split was validated by capturing the failure-test names before and after and confirming the set is IDENTICAL (zero new failures, zero accidentally-fixed). Always run `cargo fmt --check`, the `mvp_contract` suite, a sensitive-content scan, and the failure-set diff before committing.

## Remaining Work

- Continue shrinking `src/commands.rs`.
- Next candidate is the `/doctor` + `/init` domain. It is bigger and more coupled than goal/diagnose (~1.2k lines, wide dependency surface, many test couplings), so it needs the precise recipe below rather than a one-shot `sed`:
  - It is two contiguous ranges with the env helpers kept in between. Range A = `handle_doctor` through `doctor_next_actions` (this range also physically contains `handle_init`, a thin wrapper that just delegates to `handle_doctor`, plus the `DoctorOptions`/`DoctorReport`/`DoctorProviderStatus`/`DoctorShellSection`/`DoctorShellCommandStatus`/`DoctorEnvironmentSection`/`DoctorFixReport` structs). Range B = `ProviderReadinessReport`/`ProviderProbeReport` structs + impls through `provider_readiness_reports`/`_report`, `probe_provider`, `record_provider_probe`, `elapsed_ms`, `provider_type_is_implemented`, `default_provider_model`, `default_provider_endpoint`.
  - KEEP in `src/commands.rs` (env/shared, interleaved between A and B): `environment_next_actions`, `default_environment_next_actions`, `shell_command_from_slash_command`, `dedup_preserve_order`. The new module owns `/doctor` and `/init` together.
  - Re-export non-test: `pub(crate) use doctor::{handle_doctor, handle_init};` — `handle_doctor` is also reached from `src/commands/diagnose.rs` via `super::handle_doctor`, so the re-export keeps that path resolving.
  - Re-export `#[cfg(test)]` + make `pub(crate)` (these are called directly by `commands.rs` tests): `parse_doctor_options`, `apply_doctor_fixes`, `doctor_next_actions`, `doctor_shell_next_actions`, `expected_deepcli_workspace_paths`, `format_shell_command_status`, `shell_command_status_in`, `provider_readiness_reports`, `record_provider_probe`, and the structs `DoctorOptions` and `ProviderProbeReport` (tests construct these, so give them `pub(crate)` fields).
  - `super::` imports needed: `build_git_identity_report`, `format_git_identity_summary`, `git_identity_json`, `GitIdentityReport`, `command_names`/`CommandRouter`, `dedup_preserve_order`, `environment_next_actions`, `local_action_checklist`, and the completion types it reports on (`CompletionFormat`, `CompletionStatusReport`).
  - `crate::` imports needed: `config::AppConfig`, `providers::{create_provider, ChatRequest, ProviderMessage}`, `session::{SessionStore, SessionMetadata}`, `tools::{ToolExecutor, DiscoveredTestCommand, EnvironmentReport}`, `workspace::WorkspaceManager`, plus `anyhow::{bail, Context, Result}`, `chrono::Utc`, `serde_json::{json, Value}`, `std::{fs, path::{Path, PathBuf}, time::Instant}`. (`provider_readiness` does live provider probing, so confirm the providers import set against the compiler.)
- Avoid moving cross-coupled `/env`, `/verify`, `/handoff`, or the scorecard/round/recipes/benchmark cluster without a tighter plan (`/recipes` depends on `build_round_report` and benchmark baseline helpers; `build_git_identity_report` is shared between scorecard and diagnose/doctor and must stay in `src/commands.rs`).
- Update `docs/MODULES/commands.md` and `tests/mvp_contract.rs` for every new split module.
- Preserve the red-green flow: add the ownership contract first, observe the expected missing-file failure, then move code.

## Push Checklist

Before pushing or opening a PR:

- `git status --short`
- `cargo fmt --check`
- `cargo test commands::tests --lib` (compare the failing-test set against the 23-failure Windows baseline; do not introduce new failures)
- `cargo test --test mvp_contract`
- `./scripts/deepcli preflight --quick --json`
- `git diff --cached --check` when there are staged changes
- scan staged or outgoing changes for local artifacts, credentials, sessions, logs, benchmark evidence, support bundles, and sensitive-looking tokens
- expected commit identity `zero-kotori <kotorizero8@gmail.com>`

Known limitation: `preflight --quick` intentionally skips clippy and gate. The 23 baseline test failures are environment-dependent (POSIX shell / git / cargo execution on Windows), not actionable refactor regressions.
