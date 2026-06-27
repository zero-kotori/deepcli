# Tools Module

## Responsibility

`src/tools.rs` owns tool declarations and execution for file reads/writes, patching, shell, Git, tests, environment checks/setup, web search, terminal launch, prompt/skill helpers, and subagent spawning. Each `ToolDeclaration` owns the provider parameter schema used to build `ToolSpec` plus the default permission surface, write, and network metadata used to build `ToolRequest`.

## Boundaries

- Tools must not bypass `src/permissions.rs` for write, shell, Git, network, Docker, terminal, or setup actions.
- Tool declarations, argument schemas, permission surfaces, and audit lifecycle should remain part of the typed declaration contract.
- Primary tool execution paths should evaluate permissions through `ToolDeclaration::permission_request` with `ToolPermissionContext`; explicit filesystem helpers are reserved for file operations and file sub-operations.
- Command handlers and runtime should call tools through the registry/executor instead of duplicating tool behavior.
- Local benchmark artifacts and support bundles remain ignored workspace evidence and must not be committed.

## Tests

- `cargo test mvp_tool_registry_exposes_required_tools --test mvp_contract`
- `cargo test tool_declarations_own_provider_schema --test mvp_contract`
- `cargo test tool_declarations_build_permission_requests --test mvp_contract`
- Focused `tools::tests::*` for path safety, approvals, patching, shell/test execution, prompt/skill/subagent helpers, and environment actions.
- Command JSON tests for `/test`, `/env`, `/git`, `/terminal`, and related reports.

## Documentation Sync

Update this file when tool ownership, permission surface, argument contract, or audit lifecycle changes. Update `docs/COMMANDS.md` when a tool-backed command changes its public behavior.
