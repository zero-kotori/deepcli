# deepcli 架构 Harness

本 harness 是给在 deepcli 上工作的 agent 提供的轻量工程上下文。它不是 fake-provider 运行器，也不规定固定的修改路径；它的职责是在后续代码改动之前，把模块所有权、边界原则、文档同步与验证要求显式呈现出来。

## 模块地图

| 模块 | 所有权文档 | 当前职责 | 当前风险 |
|---|---|---|---|
| `src/commands.rs` | `docs/MODULES/commands.md` | 命令分发、共享命令 helper、Git-identity 报告，以及把各命令委派给 `src/commands/*.rs` 子模块。 | 仍是最大热点，但命令 handler 已全部拆出到子模块；剩余主要是分发、共享 helper 和大型文件内测试模块。 |
| `src/runtime.rs` | `docs/MODULES/runtime.md` | Agent loop、provider turn、工具调用循环、上下文组装、会话观测。 | runtime、observation 与 provider-turn 关注点仍纠缠在一起。 |
| `src/tools.rs` | `docs/MODULES/tools.md` | 文件、shell、Git、测试、环境、web、prompt、skill、子 agent 等工具的声明与执行。 | 工具声明、权限面与审计生命周期需要更强的类型化契约。 |
| `src/session.rs` | `docs/MODULES/session.md` | 持久化会话、元数据、消息、审计事件、plan、goal、审批、旁路问题、测试、diff、备份。 | 多模块依赖会话结构，schema 改动需注意迁移。 |
| `src/permissions.rs` | `docs/MODULES/permissions.md` | 文件系统、shell、Git、网络、Docker、终端、setup 等操作的权限决策。 | 工具的写入/高风险操作不得绕过本层。 |
| `src/ui.rs` | `docs/MODULES/ui.md` | TUI 状态、消息框、monitor tab、running-safe 命令分发与渲染。 | UI 仍承载过多投影与交互逻辑；后续改动应消费领域 projection。 |

其它支撑模块：

- `src/cli.rs` 负责进程入口、provider 别名、one-shot 路由与交互模式选择。
- `src/providers.rs` 负责 provider 适配器与 provider 能力映射。
- `src/config.rs` 负责有效配置与 provider 凭证引用。
- `src/workspace.rs` 负责工作区授权与上下文源过滤。
- `src/privacy.rs` 负责脱敏与隐私发现逻辑。
- `src/prompts.rs`、`src/skills.rs`、`src/agents.rs` 负责本地库元数据。

`src/commands.rs` 的命令 handler 现已拆分到按命令/领域划分的子模块：`src/commands/<name>.rs`（如 `goal`、`diagnose`、`doctor`、`recipes`、`opportunities`、`productloop`、`session`、`env`、`delivery` 等）。子模块通过 `super::` 复用 `src/commands.rs` 中的共享 helper，`src/commands.rs` 通过 `pub(crate) use` re-export 各 handler 与跨模块 helper。

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

`tests/mvp_contract.rs::architecture_harness_docs_cover_commands_and_modules` 检查第一层 docsync：harness 章节、命令分组表、模块 owner 文档是否齐备。`tests/mvp_contract.rs::command_docs_match_registry` 进一步校验 `docs/COMMANDS.md` 的命令清单与分组与命令 registry（`CommandRouter::help_summaries`）逐项一致，新增/删除命令或改动分组时若未同步该文档即会失败。

## 验证

用能证明所改表面的最小命令，提交前再逐步放大：

- 仅改 harness 文档：`cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract`。
- 命令 registry、help、别名或 running-safe 改动：上面的 harness 文档测试 + `cargo test mvp_slash_commands_are_registered --test mvp_contract`。
- 工具契约改动：工具单测 + `cargo test mvp_tool_registry_exposes_required_tools --test mvp_contract`。
- runtime 或 session 行为改动：所改模块的聚焦单测 + 受影响命令的契约测试。
- UI projection 改动：聚焦的 `ui::tests::*` projection 或交互测试。
- 提交前检查点：至少 `cargo fmt --check`、`cargo test`、隐私扫描和 `git diff --check`；用 `./scripts/deepcli preflight --quick --json` 做快速本地 gate。

完整产品循环证据仍保留在本地 `.deepcli/benchmarks/`，不应提交。
