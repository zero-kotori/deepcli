# Session Module

## Responsibility

`src/session.rs` owns persisted session metadata and records: messages, audit events, plans, goals, approvals, side questions, tool calls, tests, diffs, backups, and session lookup by id or prefix.

## Boundaries

- Other modules should use `SessionStore` and session model methods instead of ad hoc file writes.
- Schema changes need migration or backwards-compatible readers.
- Runtime can append observations and state changes, but command and UI surfaces should consume session APIs rather than parse storage files directly.
- Fork, resume, goal, plan, approval, and by-the-way flows should preserve persisted-context semantics.

## Tests

- Focused `session::tests::*` for storage, prefix lookup, metadata, records, and backwards compatibility.
- Command contract tests for `/session`, `/resume`, `/fork`, `/goal`, `/plan`, `/approval`, and `/btw`.
- UI tests for running-safe session projections.

## Documentation Sync

Update this file when session storage shape, selection rules, fork/resume semantics, or migration rules change. Update `docs/COMMANDS.md` for public command or schema changes and `docs/ai/CONTEXT.md` for major product decisions.
