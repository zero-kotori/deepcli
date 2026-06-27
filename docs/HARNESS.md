# deepcli Architecture Harness

This harness is a lightweight engineering context for agents working on deepcli. It is not a fake-provider runner and it does not prescribe a fixed edit path. Its job is to make module ownership, boundary rules, documentation sync, and verification expectations visible before future code changes.

## Module Map

| Module | Owner Doc | Current Responsibility | Current Risk |
|---|---|---|---|
| `src/commands.rs` | `docs/MODULES/commands.md` | Slash command parsing, help, command handlers, JSON reports, scorecard, benchmark, session and support workflows. | Largest hotspot; new command logic should move toward registries or domain modules instead of adding more body logic here. |
| `src/runtime.rs` | `docs/MODULES/runtime.md` | Agent loop, provider turns, tool-call loop, context assembly, session observation. | Runtime, observation, and provider-turn concerns are still close together. |
| `src/tools.rs` | `docs/MODULES/tools.md` | Tool declarations and execution for files, shell, Git, tests, environment, web, prompt, skill, and subagent actions. | Tool declarations, permissions surface, and audit lifecycle need a stronger typed contract. |
| `src/session.rs` | `docs/MODULES/session.md` | Persisted sessions, metadata, messages, audit events, plans, goals, approvals, side questions, tests, diffs, and backups. | Many modules depend on session shape; schema changes need migration care. |
| `src/permissions.rs` | `docs/MODULES/permissions.md` | Permission decisions for filesystem, shell, Git, network, Docker, terminal, and setup actions. | Tools must not bypass this layer for write or high-risk operations. |
| `src/ui.rs` | `docs/MODULES/ui.md` | TUI state, message box, monitor tabs, running-safe command dispatch, and rendering. | UI still owns too much projection and interaction logic; future changes should consume domain projections. |

Other supporting modules:

- `src/cli.rs` owns process entry, provider aliases, one-shot routing, and interactive mode selection.
- `src/providers.rs` owns provider adapters and provider capability mapping.
- `src/config.rs` owns effective config and provider credential references.
- `src/workspace.rs` owns workspace authorization and context source filtering.
- `src/privacy.rs` owns redaction and privacy finding logic.
- `src/prompts.rs`, `src/skills.rs`, and `src/agents.rs` own local library metadata.

## Boundary Principles

- Commands may parse CLI-like input and build reports, but durable domain behavior should move toward owned modules or registries.
- UI should render state and collect user input; it should not become the source of truth for command, tool, session, or permission behavior.
- Runtime should orchestrate provider turns and tool loops; it should not format UI text or reach around session APIs.
- Tools must go through permission decisions for write, shell, Git, network, Docker, terminal, and setup surfaces.
- Session data should be mutated through `SessionStore` and session model methods, not by ad hoc file writes in unrelated modules.
- Stable JSON schemas need a clear owner and tests before their shape changes.
- Support and legacy aliases should remain thin wrappers over canonical commands.
- Context compression and LLM wiki behavior are out of scope for this harness refactor until a separate plan is written.

## Documentation Sync

Update documentation in the same change when behavior moves:

- Command names, aliases, groups, running-safe status, stable JSON schemas, or public output contracts: update `docs/COMMANDS.md`.
- Module responsibility, boundaries, tests, or sync rules: update the matching `docs/MODULES/*.md` file and this harness if the module map changes.
- Core product scope or current handoff decisions: update `docs/ai/CONTEXT.md`.
- Architecture decisions that are hard to reverse: add or update an ADR under `docs/ADR/`.
- Removed, downgraded, or legacy behavior: remove old promises from user-facing docs or mark the entry support/legacy.

`tests/mvp_contract.rs::architecture_harness_docs_cover_commands_and_modules` checks the first docsync layer: harness sections, command table coverage, command groups, and module owner documents.

## Verification

Use the smallest command that proves the edited surface, then broaden before commits:

- Documentation-only harness edits: `cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract`.
- Command registry, help, alias, or running-safe edits: the harness doc test plus `cargo test mvp_slash_commands_are_registered --test mvp_contract`.
- Tool contract edits: tool unit tests plus `cargo test mvp_tool_registry_exposes_required_tools --test mvp_contract`.
- Runtime or session behavior edits: focused unit tests in the touched module plus affected command contract tests.
- UI projection edits: focused `ui::tests::*` projection or interaction tests.
- Pre-commit checkpoint: at minimum `cargo fmt --check`, `cargo test`, a privacy scan, and `git diff --check`; use `./scripts/deepcli preflight --quick --json` for the fast local gate.

Full product-loop evidence remains local under `.deepcli/benchmarks/` and should not be committed.
