# deepcli 架构 Harness

本 harness 是给在 deepcli 上工作的 agent 提供的轻量工程上下文。它不是 fake-provider 运行器，也不规定固定的修改路径；它的职责是在后续代码改动之前，把模块所有权、边界原则、文档同步与验证要求显式呈现出来。

## 模块地图

| 模块 | 所有权文档 | 当前职责 | 当前风险 |
|---|---|---|---|
| `src/commands.rs` | `docs/MODULES/commands.md` | 命令分发、跨模块 re-export，以及把各命令委派给 `src/commands/*.rs` 子模块。 | 命令 handler、大型测试模块、delivery diff projection、session catalog owner、session restore-backup owner、session selection owner、Git identity 报告、无状态共享 helper、会话共享 helper、环境 nextActions helper、command group/legacy policy projection、action checklist projection、scorecard report builder、scorecard opportunity projection、benchmark dispatch、round benchmark gate projection、round goal status projection、round report builder、benchmark artifact projection、benchmark presets catalog、benchmark run/record/run-suite execution artifact、benchmark baseline projection、benchmark history projection 和 benchmark status handler/schema/freshness projection 已拆出；命令删除/降级审计已在 `docs/COMMANDS.md` 落地并由契约测试覆盖，剩余风险主要是部分大命令模块后续仍可继续瘦身。 |
| `src/runtime.rs` | `docs/MODULES/runtime.md` | Agent loop、provider turn、工具调用循环、上下文组装、会话观测。 | 本轮按计划只标记 runtime 边界并保持现有上下文行为；provider-turn、tool loop 与 observation 深拆留给后续单独计划。 |
| `src/tools.rs` | `docs/MODULES/tools.md` | 文件、shell、Git、测试、环境、web、prompt、skill、子 agent 等工具的声明与执行。 | `ToolDeclaration` / `ToolRegistry` / provider schema / `permission_request` 契约已落地并由 contract tests 覆盖；后续可继续增强审计生命周期类型化。 |
| `src/session.rs` | `docs/MODULES/session.md` | 持久化会话、元数据、消息、审计事件、plan、goal、审批、旁路问题、测试、diff、备份。 | 多模块依赖会话结构，schema 改动需注意迁移。 |
| `src/permissions.rs` | `docs/MODULES/permissions.md` | 文件系统、shell、Git、网络、Docker、终端、setup 等操作的权限决策。 | 工具的写入/高风险操作不得绕过本层。 |
| `src/ui.rs` / `src/ui/native_terminal.rs` / `src/ui/resume_picker.rs` | `docs/MODULES/ui.md` | 原生终端聊天入口、输入编辑、低噪声流式对话、失败/审批状态、plan 采访文本选项和文本 resume picker。 | Fullscreen TUI/Ratatui 已删除；后续 UI 改动应围绕 native terminal 交互体验和真实 pty smoke 验证。 |

其它支撑模块：

- `src/cli.rs` 负责进程入口、provider 别名、one-shot 路由与交互模式选择。
- `src/providers.rs` 负责 provider 适配器与 provider 能力映射。
- `src/config.rs` 负责有效配置与 provider 凭证引用。
- `src/workspace.rs` 负责工作区授权与上下文源过滤。
- `src/privacy.rs` 负责脱敏与隐私发现逻辑。
- `src/prompts.rs`、`src/skills.rs`、`src/agents.rs` 负责本地库元数据。
- `src/schema_ids.rs` 是稳定 JSON schema 标识符（`deepcli.<name>.v1`）的所有权 registry，生产发射点统一从这里取常量，测试断言保留字面量作为独立值锚点。

`src/commands.rs` 的命令 handler 现已拆分到按命令/领域划分的子模块：`src/commands/<name>.rs`（如 `goal`、`diagnose`、`doctor`、`recipes`、`opportunities`、`productloop`、`session`、`env`、`delivery` 等）。子模块通过 `super::` 复用命令层 re-export 的跨模块 helper；无状态共享 helper 已拆到 `src/commands/shared.rs`，会话列表/活动/状态/存储 helper 已拆到 `src/commands/session_helpers.rs`，环境 nextActions helper 已拆到 `src/commands/environment_actions.rs`，action checklist/filter/label 投影已拆到 `src/commands/action_checklist.rs`，scorecard report builder、scorecard category projection、scorecard summary JSON 与 scorecard text/JSON output 已拆到 `src/commands/scorecard_report.rs`，scorecard opportunities、opportunity projection、recommended opportunity 与 opportunity counts 已拆到 `src/commands/scorecard_opportunities.rs`，benchmark dispatch、scorecard-compatible benchmark args 与 benchmark gate dispatch 已拆到 `src/commands/benchmark_dispatch.rs`，round benchmark gate projection、benchmark trend gate、round benchmark status projection 与 freshness suffix 已拆到 `src/commands/round_benchmark_gates.rs`，round goal status、goal readiness projection、goalStatus JSON 与 goal_readiness gate support 已拆到 `src/commands/round_goal_status.rs`，round report builder、round text/JSON output、round summary JSON 与 round benchmark suite wrapper 已拆到 `src/commands/round_report.rs`，benchmark artifact list/show/cleanup 与 artifact projection 已拆到 `src/commands/benchmark_artifacts.rs`，benchmark presets catalog 与 required/default preset projection 已拆到 `src/commands/benchmark_presets.rs`，benchmark run/record/run-suite execution artifact、shell timeout、suite schema 与 artifact path slug 已拆到 `src/commands/benchmark_runs.rs`，benchmark baseline-template/inventory/compare-ready projection 已拆到 `src/commands/benchmark_baselines.rs`，benchmark summary/trends/compare history projection 与 trend gate 状态已拆到 `src/commands/benchmark_history.rs`，benchmark status handler、status schema、freshness projection 与 required preset 覆盖状态已拆到 `src/commands/benchmark_status.rs`，`src/commands.rs` 仅保留 `pub(crate) use` 兼容现有 sibling module 调用。大型命令契约测试已外置为 `src/commands/tests.rs`，入口文件只保留 `#[cfg(test)] mod tests;`。delivery diff projection、path scope filtering、session diff fallback 与 diff stat/name-only projection 已拆到 `src/commands/delivery_diff.rs`，delivery report builder、verification report projection、handoff report projection 与 delivery report JSON 已拆到 `src/commands/delivery_reports.rs`，session catalog owner、session list/search projection、session catalog JSON 与 prune-empty report 已拆到 `src/commands/session_catalog.rs`，session restore-backup owner、restore-backup dry-run 与 restore preview JSON 已拆到 `src/commands/session_restore.rs`，session inspect owner、session record projection、session inspect JSON 与 tools/tests/diffs/backups projection 已拆到 `src/commands/session_inspect.rs`，session recovery owner、session next/diagnose projection 与 next-action signals 已拆到 `src/commands/session_recovery.rs`，session export owner、session export JSON 与 export path safety 已拆到 `src/commands/session_export.rs`，session rename owner、session rename parser 与 session title update 已拆到 `src/commands/session_rename.rs`，session resumable owner、可恢复会话筛选与 workspace fallback 已拆到 `src/commands/session_resumable.rs`，session selection owner、inspection fallback、scoped action parser 与 approval/BTW cross-session lookup 已拆到 `src/commands/session_selection.rs`。Git identity 报告和只读 Git stdout helper 已拆到 `src/commands/git_identity.rs`，由 doctor、selftest、privacy、preflight 和 product loop 复用。

`src/commands/registry.rs` 拥有显式 `CommandMetadata` 表，记录每个公开命令的分组和 running-safe 标记；同文件还拥有 legacy successor metadata、slash alias metadata（如 `/login`、`/accept`、`/cleanup`）与 completion-only alias metadata（如 provider preset、`ask`、`sessions`、legacy `repl`）。`src/commands/command_policy.rs` 将 core/support/legacy/experimental 的 visibility/policy、slash legacy successor/policy 和 completion-only legacy alias successor/policy 投影为机器可读 JSON，供 completion catalog 与外部 UI 消费。parser、`help_summaries`、补全目录、UI running-safe 提示和文档同步测试应从这些 registry/policy metadata 派生命令元数据，避免 parser、help、completion、UI 与 docs 各自维护分类。每个 `legacy` slash 命令或 completion-only legacy alias 必须在 registry 中记录 successor/policy；公开 slash 命令还必须在 `docs/COMMANDS.md` 行内写明替代入口。

Delivery review heuristic、review risk detection 与 sensitive/dangerous/panic-prone finding projection 已拆到 `src/commands/delivery_review.rs`；delivery verify/handoff owner、verify/handoff option parser、test/env execution helper 与 verification session selection 已拆到 `src/commands/delivery_verify.rs`；`src/commands/delivery.rs` 只保留 `/diff`、`/review` 的命令编排和 diff/review owner 委派。

## 边界原则

- 命令层可以解析类 CLI 输入并构建报告，但持久的领域行为应迁移到有所有权的模块或 registry。
- UI 应渲染状态、收集用户输入；不应成为命令、工具、会话或权限行为的真理来源。
- runtime 应编排 provider turn 与工具循环；不应拼接 UI 文案或绕过 session API。
- 工具的写入、shell、Git、网络、Docker、终端、setup 等操作必须经过权限决策。
- 会话数据应通过 `SessionStore` 与会话模型方法修改，不应在无关模块里临时写文件。
- 稳定 JSON schema 在改形状前需要明确的 owner 和测试。
- support/legacy 别名应保持为对规范命令的薄 wrapper。
- 上下文压缩与 LLM wiki 行为本轮 harness 重构不在范围内，待单独计划。

## 文档同步

行为迁移时，应在同一次改动中同步文档：

- 命令名、别名、分组、running-safe 状态、稳定 JSON schema 或公开输出契约：更新 `docs/COMMANDS.md`。
- 模块职责、边界、测试或同步规则：更新对应 `docs/MODULES/*.md`；若模块地图变化，同时更新本 harness。
- 核心产品范围或当前 handoff 决策：更新 `docs/ai/CONTEXT.md`。
- 难以回退的架构决策：在 `docs/ADR/` 下新增或更新 ADR。
- 删除、降级或 legacy 行为：从面向用户的文档中删除旧承诺，或将条目标记为 support/legacy。

`tests/mvp_contract.rs::architecture_harness_docs_cover_commands_and_modules` 检查第一层 docsync：harness 章节、命令分组表、模块 owner 文档是否齐备。`tests/mvp_contract.rs::commands_entrypoint_delegates_stateless_shared_helpers` 校验无状态共享 helper 由 `src/commands/shared.rs` 拥有，且命令模块文档记录该 owner。`tests/mvp_contract.rs::commands_entrypoint_delegates_session_shared_helpers` 校验会话共享 helper 由 `src/commands/session_helpers.rs` 拥有，且命令模块文档记录该 owner。`tests/mvp_contract.rs::session_delegates_selection_owner` 校验 session selection owner 由 `src/commands/session_selection.rs` 拥有，且命令模块文档记录 `SessionFallbackKind`、scoped action parser 与 approval/BTW cross-session lookup。`tests/mvp_contract.rs::commands_entrypoint_delegates_environment_action_helpers` 校验环境 nextActions helper 由 `src/commands/environment_actions.rs` 拥有，且命令模块文档记录该 owner。`tests/mvp_contract.rs::command_policy_owner_projects_group_and_legacy_strategy` 校验 command policy owner 由 `src/commands/command_policy.rs` 拥有，并在命令模块文档中记录 group policy 与 legacy policy projection。`tests/mvp_contract.rs::command_registry_explicitly_owns_public_command_metadata` 校验每个公开 help topic 都有显式 `CommandMetadata`，且 `CommandHelpSummary` 的分组和 running-safe 标记来自该 registry。`tests/mvp_contract.rs::command_registry_owns_legacy_successor_metadata` 校验每个 `legacy` 命令都有显式 successor/policy，且 `docs/COMMANDS.md` 行内同步写明替代入口。`tests/mvp_contract.rs::command_registry_owns_slash_alias_metadata` 校验 parser 兼容 alias 也有显式 registry metadata。`tests/mvp_contract.rs::command_registry_owns_completion_alias_metadata` 校验 shell completion 使用的 provider preset 与顶层别名也有显式 registry metadata。`tests/mvp_contract.rs::command_surface_pruning_audit_covers_aliases_and_legacy_entries` 校验 `docs/COMMANDS.md` 的删除/降级审计覆盖 parser thin alias、legacy slash successor 和 completion-only alias。`tests/mvp_contract.rs::command_docs_match_registry` 进一步校验 `docs/COMMANDS.md` 的命令清单与分组与命令 registry（`CommandRouter::help_summaries`）逐项一致，新增/删除命令或改动分组时若未同步该文档即会失败。`tests/mvp_contract.rs::authoritative_docs_exist_and_cover_schema_owner` 校验权威文档（`docs/ARCHITECTURE.md`、`docs/CORE_FEATURES.md`、`docs/COMMANDS.md`、`docs/HARNESS.md`、`docs/ADR/*`）存在且非空，并校验 schema-id registry 在 `docs/CORE_FEATURES.md` 有文档入口。

## 验证

用能证明所改表面的最小命令，提交前再逐步放大：

- 仅改 harness 文档：`cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract`。
- 命令 registry、help、别名、删除/降级审计或 running-safe 改动：上面的 harness 文档测试 + `cargo test command_registry_explicitly_owns_public_command_metadata --test mvp_contract`、`cargo test command_registry_owns_slash_alias_metadata --test mvp_contract`、`cargo test command_registry_owns_completion_alias_metadata --test mvp_contract`、`cargo test command_surface_pruning_audit_covers_aliases_and_legacy_entries --test mvp_contract` 和 `cargo test command_docs_match_registry --test mvp_contract`。
- 工具契约改动：工具单测 + `cargo test mvp_tool_registry_exposes_required_tools --test mvp_contract`。
- runtime 或 session 行为改动：所改模块的聚焦单测 + 受影响命令的契约测试。
- UI 或入口边界改动：`cargo test ui::native_terminal::tests --lib`、`cargo test ui_entrypoint_is_native_terminal_only_boundary --test mvp_contract`，涉及真实终端观感时再运行 `scripts/native-terminal-smoke`。
- 提交前检查点：至少 `cargo fmt --check`、`cargo test`、隐私扫描和 `git diff --check`；用 `./scripts/deepcli preflight --quick --json` 做快速本地 gate。

完整产品循环证据仍保留在本地 `.deepcli/benchmarks/`，不应提交。
