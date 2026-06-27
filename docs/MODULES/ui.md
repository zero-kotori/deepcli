# UI Module

## Responsibility

`src/ui.rs` owns the terminal UI: message input, transcript rendering, monitor tabs, quick actions, resume picker, approval interactions, running-safe side-command dispatch, and task observation layout.

## Boundaries

- UI should render domain state and collect user intent; it should not define the canonical command, tool, permission, or session contract.
- Running-safe command dispatch must match actual handler support and command documentation.
- UI projections should come from runtime, session, and command report models rather than terminal text parsing.
- High-risk actions should be prefilled or routed through approvals instead of being triggered by ambiguous UI clicks.

## Tests

- Focused `ui::tests::*` for message input, monitor tabs, quick actions, running-safe guards, resume picker, approvals, and rendering.
- Command contract tests when UI consumes stable JSON or command checklists.
- `cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract` for docsync coverage.

## Documentation Sync

Update this file when monitor tabs, running-safe behavior, UI projection ownership, or high-risk interaction rules change. Update `docs/COMMANDS.md` when running-safe public behavior changes.
