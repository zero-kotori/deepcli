# Commands Module

## Responsibility

`src/commands.rs` currently owns command dispatch, most one-shot command handlers, stable JSON report builders, scorecard, benchmark, session-facing reports, support diagnostics, and local workflow formatting. `src/commands/completion.rs` owns shell completion command catalogs, script formatting, install/status reports, and completion output writes. `src/commands/help.rs` owns command help topics, help text formatting, topic alias normalization, and help summaries. `src/commands/logs.rs` owns `/logs` log discovery, safe tailing, redaction, text/JSON reports, and logs output writes. `src/commands/model.rs` owns `/model` show/list/set handling, model inspection JSON, provider runtime display, and project model config updates. `src/commands/parser.rs` owns slash command parsing, public slash command variants, aliases, and argument normalization. `src/commands/permissions.rs` owns `/permissions` show/set-mode handling, permission policy JSON, and project permission mode updates. `src/commands/preflight.rs` owns `/preflight` check planning, execution summaries, diagnostics, and release-check JSON. `src/commands/quickstart.rs` owns `/quickstart` onboarding checks, readiness JSON, and startup next actions. `src/commands/registry.rs` owns command group metadata, help summary types, and running-safe classification. `src/commands/response.rs` owns shared command output helpers, workspace-contained `--output` writes, and structured command exits. `src/commands/selftest.rs` owns `/selftest` readiness checks, local install diagnostics, and selftest JSON. `src/commands/timeout.rs` owns `/timeout` show/set/reset handling, timeout JSON, and provider turn timeout config updates. `src/commands/version.rs` owns `/version` text and JSON reports.

## Boundaries

- Commands may normalize aliases and validate arguments, but durable behavior should move toward the relevant domain module.
- Command handlers should not directly mutate session files except through `SessionStore`.
- Command handlers should not execute shell, Git, filesystem write, or network actions outside the tool and permission layers.
- New public commands must update `docs/COMMANDS.md` and focused command contract tests.
- Command group metadata exposed through `CommandHelpSummary` must stay synchronized with `docs/COMMANDS.md`.
- Legacy aliases should remain thin wrappers over canonical commands.

## Tests

- `cargo test mvp_slash_commands_are_registered --test mvp_contract`
- `cargo test command_specific_help_explains_usage_examples_and_notes`
- Focused command JSON contract tests in `src/commands.rs`.
- `cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract` for docsync coverage.

## Documentation Sync

Update `docs/COMMANDS.md` for command names, groups, owners, aliases, running-safe implications, stable schema ownership, or compatibility changes. Update this file if command ownership moves out of `src/commands.rs`.
