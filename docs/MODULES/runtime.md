# Runtime Module

## Responsibility

`src/runtime.rs` owns the agent loop, provider turn lifecycle, tool-call loop, context assembly, plan state updates, and session observation used by status and UI surfaces.

## Boundaries

- Runtime orchestrates provider and tool work; it should not become a command parser or UI renderer.
- Tool execution must go through `ToolExecutor` and permission checks.
- Session state changes must go through session APIs.
- Provider-specific request and stream parsing belongs in `src/providers.rs`.
- Context compression behavior is not part of the current harness refactor unless a separate plan is written.

## Tests

- Focused `runtime::tests::*` for provider-turn, tool-loop, session observation, and planning behavior.
- Command or UI tests only when runtime state is externally projected.
- `cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract` for docsync coverage.

## Documentation Sync

Update this file and `docs/HARNESS.md` when runtime ownership, event sources, observation shape, or context boundaries change. Update stable command docs when runtime changes alter user-visible JSON reports.
