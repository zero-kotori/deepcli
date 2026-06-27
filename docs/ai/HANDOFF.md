# deepcli HARNESS Refactor Handoff

Updated: 2026-06-28 (after `/goal`, `/diagnose`, `/doctor`, `/recipes`, `/opportunities`, product-loop core, and `/session` splits)

## Current Stop Point

The current stop point is after the `/goal`, `/diagnose`, `/doctor`+`/init`, `/recipes`, `/opportunities`, the product-loop core trio (`/scorecard`+`/round`+`/benchmark` → `src/commands/productloop.rs`), and the `/session` cluster (→ `src/commands/session.rs`) extractions from `src/commands.rs`. The worktree is clean after the latest commits. The active long-term goal is still the HARNESS refactor described in `docs/ai/HARNESS_REFACTOR_PLAN.md`; do not treat this handoff as completion of that goal.

`src/commands.rs` is now ~21.2k lines (down from ~36.1k — about 15k lines / 41% removed this round); the non-test code is now only ~4.8k lines (dispatch + shared helpers + the `/env` and `/diff`/`/review`/`/verify`/`/handoff` handlers), the rest being the large in-file test module.

## Recent Commits

- `d2c8105 refactor: split command session handler`
- `a6a1b8e refactor: split product-loop core into productloop module`
- `9be6096 refactor: split command opportunities handler`
- `07a14c8 refactor: split command recipes handler`
- `1fce47d refactor: split command doctor handler`
- `8de0aee refactor: split command diagnose handler`

## What Was Completed

- Added `src/commands/goal.rs` for `/goal` show/start/clear/status/gate handling, default goal contract creation and guard-plan generation, goal readiness collection, goal session selection, and goal text/JSON formatting. `build_round_goal_status` in `src/commands.rs` keeps consuming the goal logic through crate-internal re-exports (`select_goal_session`, `collect_goal_readiness`, `GoalSessionSource`, `GoalPlanReadiness`, `GoalAcceptanceEvidence`), so `/round` goal-status summaries still share the same readiness contract.
- Added `src/commands/diagnose.rs` for `/diagnose` and `/support` handling: option parsing, diagnostics report JSON, redacted support-bundle generation and artifacts, issue templates, and diagnose next actions. It delegates workspace health to the `/doctor` handler and session diagnosis to the `/session` handler through `super::`. `workspace_relative_display` deliberately stayed in `src/commands.rs` because the benchmark code also uses it; `parse_diagnose_options` is re-exported `#[cfg(test)]`-only.
- Added `src/commands/doctor.rs` for `/doctor` and `/init` handling: doctor option parsing, workspace/shell/provider/test/Git-identity health checks, shell command path resolution, provider readiness reporting and online provider probing, doctor fix application, and doctor report text/JSON formatting. `handle_doctor` and `handle_init` are re-exported non-test (and `handle_doctor` is also reached from `diagnose.rs` via `super::`). The env helpers it sits between (`environment_next_actions`, `default_environment_next_actions`, `shell_command_from_slash_command`, `dedup_preserve_order`) stayed in `src/commands.rs`; doctor imports `environment_next_actions`/`dedup_preserve_order` plus the shared Git-identity and completion helpers via `super::`. A batch of doctor helpers and the `DoctorOptions`/`ProviderProbeReport` structs are `pub(crate)` + `#[cfg(test)]`-re-exported because `commands.rs` tests call/construct them. Removing doctor also let `crate::providers::*`, `crate::workspace::WorkspaceManager`, and `serde::Serialize` drop out of the `commands.rs` import list.
- Added `src/commands/recipes.rs` for `/recipes` topic normalization, the recipe catalog, SOTA product-loop recipe state, and recipe text/JSON formatting. It is a product-loop **leaf** (nothing depends on it), so it imports the cluster internals it consumes — `build_round_report`, `sota_baseline_next_actions`, the `scorecard_*` projection helpers, `ScorecardOpportunity`, and `DEFAULT_ROUND_SCORE_THRESHOLD`/`DEFAULT_BENCHMARK_*` consts — from `src/commands.rs` via `super::`. `sota_baseline_next_actions` and its `benchmark_*_baseline_*` helpers stayed in `src/commands.rs` because scorecard also calls them; `generic_recipe_command_label` is re-exported because scorecard/usage label helpers still call it.
- Added `src/commands/opportunities.rs` for `/opportunities` option/filter parsing, scorecard product-opportunity filtering and next actions, and opportunity text/JSON formatting. Same leaf pattern as `/recipes`: imports `build_round_report`, `RoundReport`, the `scorecard_*` helpers, `ScorecardOpportunity`, and `DEFAULT_ROUND_SCORE_THRESHOLD` via `super::`.
- Added `src/commands/productloop.rs` for the product-loop **core trio** — `/scorecard`, `/round`, `/benchmark` (~7.7k lines moved as one domain unit so the round↔scorecard↔benchmark mutual dependencies stay internal). It uses `use super::*;` to pull the shared `src/commands.rs` helpers/type-aliases it needs (pragmatic for a move this large), plus explicit `anyhow`/`serde_json` imports. `src/commands.rs` re-exports the ~18 symbols other modules consume (the 3 handlers, `build_round_report`/`RoundReport`/`ScorecardOpportunity`, the `scorecard_*` projections, `sota_baseline_next_actions`, `local_action_checklist`, and the `DEFAULT_*` consts) plus a `#[cfg(test)]` block for the ~10 internals the `commands.rs` tests touch (`build_scorecard_report`, `format_round_text`, `scorecard_summary_json`, `RoundTextInput`, and the `BENCHMARK_*`/`SCORECARD_*` schema/preset consts). `RoundReport`/`RoundTextInput` fields and the `ScorecardReport`/`BenchmarkStatusReport`/`RoundGoalStatus`/`RoundGate`/`RoundBenchmarkRun`/`ScorecardOpportunity` types became `pub(crate)` because `recipes.rs`/`opportunities.rs` (now siblings, not children) and the round-text test reach them. `build_git_identity_report` + `GitIdentityReport` stayed in `src/commands.rs` (shared with `/doctor`). Follow-up cleanups: the `use super::*;` glob and `local_action_checklist` living in `productloop` are ownership smells to tidy once the dust settles.
- Added `src/commands/session.rs` for the `/session` cluster (~3.7k lines): subcommand dispatch (list/history/next/diagnose/search/rename/export/prune-empty/tools/trace/restore-backup, plus approval and side-question queues), the running-safe `/session` handler, restore-backup preview/apply, resumable-session selection and de-noising, and session activity/inspection/diagnosis JSON. Same move-as-unit pattern as productloop (`use super::*;` header). `src/commands.rs` re-exports the ~31 session helpers that `resume.rs`/`fork.rs`/`approval.rs`/`btw.rs` and the verify/handoff code consume via `super::` (`session_metadata_json`, `resolve_session_for_*`, `sessions_with_resumable_context`, `short_id`, `SessionFallbackKind`, the scoped/queue option parsers, etc.), plus a `#[cfg(test)]` block for `parse_export_args`/`parse_limit_and_session_selection`. The `ScopedListOptions`/`ScopedActionOptions`/`QueueActionOptions` struct fields became `pub(crate)` because `approval.rs`/`btw.rs` (siblings) read them. The early shared session helpers (`format_session_list`, `session_state_name`, `git_stdout`, `latest_session_with_recorded_activity`, `session_has_no_recorded_activity`) stayed in `src/commands.rs` before the cluster and are imported via `super::`.
- Kept `docs/MODULES/commands.md` synchronized with each new command owner.
- Extended `tests/mvp_contract.rs::commands_module_docs_cover_split_source_files` so `goal.rs`, `diagnose.rs`, `doctor.rs`, `recipes.rs`, `opportunities.rs`, `productloop.rs`, and `session.rs` must exist and be documented.

## Verification Method

Red-green flow per split:

- Red first: add the new `src/commands/<name>.rs` entry to `commands_module_docs_cover_split_source_files` and observe the expected failure `<file> should exist for command module ownership`.
- Green: move the code, add `mod <name>;` plus the `pub(crate) use` re-exports (gate test-only helpers behind `#[cfg(test)]` to avoid unused-import warnings in non-test builds), update `docs/MODULES/commands.md`, then the contract test passes.

Regression proof on Windows: `cargo test commands::tests --lib` reports 23 pre-existing platform-dependent failures from tests that execute real POSIX shell / git / cargo (`verify_*`, `benchmark_*`, `git_status_*`, `doctor_shell_*`, `test_run_*`, `global_diagnose_bundle_*`, `completion_install_*`, `agent_list_*`, `skill_list_*`, `round_can_run_benchmark_suite_*`, `gate_without_current_session_*`). They fail identically on a pristine `git stash -u` baseline. Each split was validated by capturing the failing-test names before and after and confirming the set is IDENTICAL (zero new failures, zero accidentally-fixed). Always run `cargo fmt --check`, the `mvp_contract` suite, a sensitive-content scan, and the failure-set diff before committing.

## Remaining Work

The cleanly-isolated command handlers, both product-loop **leaves** (`/recipes`, `/opportunities`), the product-loop **core trio** (`productloop.rs`), and the `/session` cluster (`session.rs`) are now extracted. The remaining non-test handler clusters in `src/commands.rs` are the two warned clusters:

- `/env` cluster (`handle_env` + `environment_*` / `format_environment_*` / `parse_env_options` helpers). Note several env helpers (`environment_next_actions`, `dedup_preserve_order`, etc.) are already imported by `doctor.rs` via `super::`, so re-export those from the new env module (or keep the genuinely-shared ones in `src/commands.rs`). Move-as-unit with the productloop/session playbook.
- `/diff`+`/review`+`/verify`+`/handoff` cluster — the most cross-coupled (verify consumes review + diff + test-evidence helpers; handoff consumes verify). Move the four together as one `delivery`/`review` domain module so the mutual dependencies stay internal; re-export `is_failed_or_denied_tool_call`, `format_session_diffs`, `session_has_recorded_activity`, `SessionFallbackKind`, and any diff/review helpers other modules touch. These handlers run real git/shell (several of the 23 baseline test failures live here), so lean on the failure-set diff rather than expecting those tests to pass on Windows.
- After the handlers are out, `src/commands.rs` non-test code is essentially dispatch + shared helpers; later phases (de-hardcoding registries, doc slimming, moving the giant in-file test module next to its modules) are separate `HARNESS_REFACTOR_PLAN` phases.
- Update `docs/MODULES/commands.md` and `tests/mvp_contract.rs` for every new split module.
- Preserve the red-green flow: add the ownership contract first, observe the expected missing-file failure, then move code. The proven mechanics: `sed`-extract the block(s) into the new module (don't retype), prepend the import header, mark `pub(crate)` only what the dispatch/tests/other-modules reference, gate test-only re-exports behind `#[cfg(test)]`, delete the source ranges (descending order; keep interleaved shared helpers in place), then build → fix imports with compiler guidance → run the failure-set diff.

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
