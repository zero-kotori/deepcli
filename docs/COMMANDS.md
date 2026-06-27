# deepcli Command Groups

This file is the stage-0 command grouping baseline for the harness refactor. The current compatibility strategy is conservative: keep existing public slash commands working, but steer primary documentation and future implementation toward the `core` group. `support`, `legacy`, and `experimental` commands should stay thin and should not become new ownership centers.

| Command | Group | Owner | Status | Notes |
|---|---|---|---|---|
| /help | support | commands | stable | Command discovery and topic help. |
| /version | support | commands | stable | Local metadata and support report. |
| /about | legacy | commands | stable alias | Alias for `/version`. |
| /quickstart | support | commands | stable | First-run guide and setup check. |
| /recipes | support | commands | stable | Workflow catalog; SOTA recipe remains a navigation aid. |
| /scorecard | core | commands | stable | Product capability scoring. |
| /opportunities | experimental | commands | stable | Non-blocking opportunity report. |
| /benchmark | support | commands | stable | Local benchmark evidence management; detailed subcommands stay support. |
| /round | core | commands | stable | Main product-loop gate report. |
| /selftest | support | commands | stable | Product self-check. |
| /preflight | core | commands | stable | Release and checkpoint preflight. |
| /completion | support | commands | stable | Shell completion and command catalog. |
| /init | support | commands | stable | Project bootstrap helper. |
| /status | core | commands | stable | Active session and workspace status. |
| /usage | core | commands | stable | Provider and session usage diagnostics. |
| /health | support | commands | stable alias | Shortcut for doctor and environment checks. |
| /diagnose | support | commands | stable | Workspace and session diagnostics. |
| /support | support | commands | stable | Redacted support bundle creation. |
| /doctor | support | commands | stable | Local setup and environment diagnostics. |
| /trace | core | commands | stable | Session audit event inspection. |
| /logs | support | commands | stable | Redacted log inspection. |
| /privacy | core | commands | stable | Privacy and sensitive-value scan. |
| /context | support | commands | stable | Workspace context preview. |
| /permissions | core | permissions | stable | Permission mode inspection. |
| /login | support | commands | stable alias | Credential setup shortcut. |
| /auth | legacy | commands | stable alias | Alias for credential setup. |
| /apikey | legacy | commands | stable alias | Alias for credential setup. |
| /key | legacy | commands | stable alias | Alias for credential setup. |
| /logout | support | commands | stable alias | Credential removal shortcut. |
| /credentials | core | commands | stable | Provider credential management. |
| /config | core | commands | stable | Effective config inspection and edits. |
| /timeout | support | commands | stable | Provider-turn timeout shortcut. |
| /model | core | commands | stable | Provider/model inspection and switching. |
| /provider | legacy | commands | stable alias | Alias over `/model`. |
| /use | legacy | commands | stable alias | Alias over `/model set`. |
| /switch | legacy | commands | stable alias | Alias over `/model set`. |
| /models | legacy | commands | stable alias | Alias over `/model list`. |
| /providers | legacy | commands | stable alias | Alias over `/model list`. |
| /goal | core | session | stable | Long-running goal contract and gate. |
| /plan | core | session | stable | Requirement clarification and plan draft. |
| /fork | core | session | stable | Persisted context cloning and resume verification. |
| /diff | core | commands | stable | Workspace or session diff inspection. |
| /review | core | commands | stable | Local diff risk review. |
| /accept | core | commands | stable alias | Human acceptance report over `/verify`. |
| /gate | core | commands | stable alias | Strict verification gate over `/verify`. |
| /verify | core | commands | stable | Acceptance report and blocker aggregation. |
| /handoff | core | commands | stable | Handoff and PR-ready report. |
| /test | core | tools | stable | Test discovery and execution through tool layer. |
| /env | core | tools | stable | Environment check, plan, setup, and test workflows. |
| /check | legacy | tools | stable alias | Alias over `/env check`. |
| /docker | legacy | tools | stable alias | Target-first alias over `/env`. |
| /compiler | legacy | tools | stable alias | Target-first alias over `/env`. |
| /setup | legacy | tools | stable alias | Alias over `/env setup`. |
| /install | legacy | tools | stable alias | Alias over `/env install`. |
| /git | core | tools | stable | Git inspect and controlled write actions. |
| /web | support | tools | stable | Permission-checked web search. |
| /prompt | support | commands | stable | Local prompt library. |
| /skill | support | commands | stable | Local skill library. |
| /agent | support | commands | stable | Subagent task descriptors. |
| /btw | core | session | stable | Side-question queue. |
| /approval | core | session | stable | Approval queue inspection and resolution. |
| /session | core | session | stable | Persisted session inspection and maintenance. |
| /history | legacy | session | stable alias | Alias over `/session list`. |
| /cleanup | legacy | session | stable alias | Alias over `/session prune-empty`. |
| /next | support | session | stable | Likely next action report. |
| /resume | core | session | stable | Session resume and candidate preview. |
| /rename | legacy | session | stable alias | Runtime session title rename. |
| /stop | core | runtime | stable | Stop active TUI task and keep session resumable. |
| /quit | core | ui | stable | Exit interactive session. |
| /terminal | core | tools | stable | Open or preview same-workspace terminal. |

Compatibility notes:

- `legacy` means "keep compatible unless explicitly removed later"; it does not mean broken or deprecated today.
- New small slash commands and top-level aliases are frozen unless they directly serve the core harness refactor or an already-confirmed core capability.
- Stable JSON schemas should keep their existing version unless a migration plan and tests are added in the same change.
- Running-safe status is still sourced from code; this document records grouping and ownership, not live TUI dispatch behavior.
