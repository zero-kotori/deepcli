# deepcli Agent Context

This repository is the active workspace for the deepcli product loop. Prefer the current worktree over prior chat memory.

## Project Goal

Continue iterating deepcli toward a SOTA local-first AI coding CLI:

1. Act as a product designer and identify missing, inconvenient, or underpowered product behavior.
2. Act as an engineer and implement the highest-value improvement.
3. Verify with focused tests and product gates.
4. Repeat the loop; do not treat one iteration as final completion.

## Current Operating Rules

- Default user-facing language is Chinese.
- Keep changes scoped to the current product gap.
- Use the current repository state as authoritative before editing.
- Do not leave local benchmark/export artifacts in the worktree.
- Keep credentials, logs, sessions, and generated local evidence out of commits.
- Expected Git commit identity: `zero-kotori <kotorizero8@gmail.com>`.
- Before committing, run relevant tests plus privacy/name scans described in `docs/ai/CONTEXT.md`.

## Product Context

- Product documentation lives in `README.md`, `docs/FEATURES.md`, `docs/ai/REQUIREMENTS.md`, and `docs/ai/TECHNICAL_PLAN.md`.
- Current handoff context lives in `docs/ai/CONTEXT.md`.
- The product loop commands are `deepcli scorecard --json`, `deepcli round --json`, and `deepcli round --json --run-benchmark --fail-on-command`.
- Benchmark evidence is intentionally local under `.deepcli/benchmarks/` and should not be committed.
