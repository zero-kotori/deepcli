# deepcli HARNESS Refactor Handoff

Updated: 2026-06-28 (after `/goal`, `/diagnose`, and `/doctor` command-handler splits)

## Current Stop Point

The current stop point is after the `/goal`, `/diagnose`, and `/doctor`+`/init` command-handler extractions from `src/commands.rs`. The worktree is clean after the latest commits. The active long-term goal is still the HARNESS refactor described in `docs/ai/HARNESS_REFACTOR_PLAN.md`; do not treat this handoff as completion of that goal.

`src/commands.rs` is now ~33.5k lines (down from ~36.1k) and remains the largest complexity hotspot.

## Recent Commits

- `1fce47d refactor: split command doctor handler`
- `8de0aee refactor: split command diagnose handler`
- `29ddd35 refactor: split command goal handler`
- `60e0772 refactor: split command resume handler`
- `2c32aca refactor: split command git handler`
- `09c18d6 refactor: split command web handler`

## What Was Completed

- Added `src/commands/goal.rs` for `/goal` show/start/clear/status/gate handling, default goal contract creation and guard-plan generation, goal readiness collection, goal session selection, and goal text/JSON formatting. `build_round_goal_status` in `src/commands.rs` keeps consuming the goal logic through crate-internal re-exports (`select_goal_session`, `collect_goal_readiness`, `GoalSessionSource`, `GoalPlanReadiness`, `GoalAcceptanceEvidence`), so `/round` goal-status summaries still share the same readiness contract.
- Added `src/commands/diagnose.rs` for `/diagnose` and `/support` handling: option parsing, diagnostics report JSON, redacted support-bundle generation and artifacts, issue templates, and diagnose next actions. It delegates workspace health to the `/doctor` handler and session diagnosis to the `/session` handler through `super::`. `workspace_relative_display` deliberately stayed in `src/commands.rs` because the benchmark code also uses it; `parse_diagnose_options` is re-exported `#[cfg(test)]`-only.
- Added `src/commands/doctor.rs` for `/doctor` and `/init` handling: doctor option parsing, workspace/shell/provider/test/Git-identity health checks, shell command path resolution, provider readiness reporting and online provider probing, doctor fix application, and doctor report text/JSON formatting. `handle_doctor` and `handle_init` are re-exported non-test (and `handle_doctor` is also reached from `diagnose.rs` via `super::`). The env helpers it sits between (`environment_next_actions`, `default_environment_next_actions`, `shell_command_from_slash_command`, `dedup_preserve_order`) stayed in `src/commands.rs`; doctor imports `environment_next_actions`/`dedup_preserve_order` plus the shared Git-identity and completion helpers via `super::`. A batch of doctor helpers and the `DoctorOptions`/`ProviderProbeReport` structs are `pub(crate)` + `#[cfg(test)]`-re-exported because `commands.rs` tests call/construct them. Removing doctor also let `crate::providers::*`, `crate::workspace::WorkspaceManager`, and `serde::Serialize` drop out of the `commands.rs` import list.
- Kept `docs/MODULES/commands.md` synchronized with each new command owner.
- Extended `tests/mvp_contract.rs::commands_module_docs_cover_split_source_files` so `src/commands/goal.rs`, `src/commands/diagnose.rs`, and `src/commands/doctor.rs` must exist and be documented.

## Verification Method

Red-green flow per split:

- Red first: add the new `src/commands/<name>.rs` entry to `commands_module_docs_cover_split_source_files` and observe the expected failure `<file> should exist for command module ownership`.
- Green: move the code, add `mod <name>;` plus the `pub(crate) use` re-exports (gate test-only helpers behind `#[cfg(test)]` to avoid unused-import warnings in non-test builds), update `docs/MODULES/commands.md`, then the contract test passes.

Regression proof on Windows: `cargo test commands::tests --lib` reports 23 pre-existing platform-dependent failures from tests that execute real POSIX shell / git / cargo (`verify_*`, `benchmark_*`, `git_status_*`, `doctor_shell_*`, `test_run_*`, `global_diagnose_bundle_*`, `completion_install_*`, `agent_list_*`, `skill_list_*`, `round_can_run_benchmark_suite_*`, `gate_without_current_session_*`). They fail identically on a pristine `git stash -u` baseline. Each split was validated by capturing the failing-test names before and after and confirming the set is IDENTICAL (zero new failures, zero accidentally-fixed). Always run `cargo fmt --check`, the `mvp_contract` suite, a sensitive-content scan, and the failure-set diff before committing.

## Remaining Work

The cleanly-isolated command handlers are now largely extracted. The remaining bulk of `src/commands.rs` is two intertwined masses plus the warned clusters, and each needs a domain-level plan rather than a one-shot handler move:

- Product-loop cluster (`handle_recipes`, `handle_scorecard`, `handle_opportunities`, `handle_round`, `handle_benchmark` and their helpers, roughly the first ~8k lines of `src/commands.rs`). These are deeply intertwined: `round` consumes `scorecard` + `benchmark`, `recipes` consumes `round` + benchmark-baseline helpers, `scorecard` consumes `benchmark`. Extract as a domain unit (e.g. a `src/commands/productloop/` submodule or one module per stable schema owner) rather than per-handler, and keep the stable JSON schema owners (`deepcli.scorecard.v1`, `deepcli.round.v1`, `deepcli.benchmark.*`) intact. `build_git_identity_report` is shared with `doctor` and must stay in `src/commands.rs`.
- Session cluster (`handle_session` and its ~50 helpers). Large and cross-coupled: `src/commands/resume.rs` and `src/commands/fork.rs` already import many session helpers (`session_metadata_json`, `session_state_name`, `resolve_session_for_*`, `sessions_with_resumable_context`, `latest_session_with_recorded_activity`, etc.) from `src/commands.rs` via `super::`. Moving `handle_session` means deciding which shared helpers move with it (and get re-exported back) versus stay; plan the shared-helper boundary first.
- Warned clusters: `/env`, and `/diff`+`/review`+`/verify`+`/handoff`. The verify/handoff code shares diff/review/test-evidence helpers and is the most cross-coupled; needs its own scoped plan before moving.
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
