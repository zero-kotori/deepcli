# Permissions Module

## Responsibility

`src/permissions.rs` owns permission mode evaluation and risk decisions for filesystem, shell, Git, network, Docker, terminal, and setup operations.

## Boundaries

- Tools and runtime must consult permission decisions before high-risk or write operations.
- Commands may explain permission outcomes, but policy decisions belong here.
- Approval and audit records should reflect the same surface and risk classification used by the permission engine.
- New risk surfaces need tests before being exposed through tools or commands.

## Tests

- Focused `permissions::tests::*` for read-only shell, destructive shell, Docker, package install, and medium-risk decisions.
- Tool tests that verify pending approval and assume-yes behavior.
- Command tests for permission reporting when public JSON output changes.

## Documentation Sync

Update this file when risk classification, approval requirements, or permission surfaces change. Update `docs/COMMANDS.md` when `/permissions` or tool-backed command behavior changes.
