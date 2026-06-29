# deepcli 架构

本文描述 deepcli 当前的真实架构与分层。模块所有权与边界细节见 `docs/HARNESS.md` 和 `docs/MODULES/*.md`；命令面见 `docs/COMMANDS.md`；核心功能契约见 `docs/CORE_FEATURES.md`。

## 分层

deepcli 是一个 Rust CLI（`src/main.rs` 二进制 + `src/lib.rs` 库），外层是 `scripts/deepcli` 启动 wrapper。请求自上而下流经：

```
scripts/deepcli (wrapper)         顶层命令/别名 → slash，自动构建二进制、注入 -C/--yes/凭据
  └─ src/cli.rs                   参数解析、provider/模式归一化、one-shot 路由、交互入口选择
       ├─ src/commands/*          slash 命令解析(parser)、分发、各命令 handler、稳定 JSON 报告
       ├─ src/runtime.rs          Agent loop、provider turn、工具调用循环、会话观测
       │    ├─ src/providers.rs   Provider 适配(DeepSeek/Kimi)、流式、tool call、usage、重试、代理
       │    ├─ src/tools/*        工具声明与执行(文件/shell/git/test/env/web/...)，经权限层
       │    └─ src/permissions.rs 权限模式、sandbox、风险分级、审批
       ├─ src/session.rs          会话持久化(消息/工具/审计/plan/goal/diff/backup/审批/旁路问题)
       └─ src/ui.rs               TUI：message box、任务观察面板、running-safe 分发与渲染
```

支撑模块：`src/config.rs`(有效配置、serde 默认与凭据引用)、`src/workspace.rs`(工作区授权与上下文过滤)、`src/privacy.rs`(脱敏)、`src/prompts.rs`/`src/skills.rs`/`src/agents.rs`(本地库)、`src/schema_ids.rs`(稳定 JSON schema 标识符的所有权 registry)。

## 关键边界原则

- Agent 不直接碰文件系统/shell/网络/Git，一切经工具系统 + 权限引擎。
- 命令层可解析输入并构建报告，但持久领域行为应落在有所有权的模块或 registry。
- UI 渲染状态、收集输入，不作为命令/工具/会话/权限行为的真理来源（收束方向：UI 消费领域 projection）。
- runtime 编排 provider turn 与工具循环，不拼 UI 文案、不绕过 `SessionStore`。
- 工具的写入/shell/Git/网络/Docker/终端/setup 操作必须经权限决策。
- 稳定 JSON schema 由 `src/schema_ids.rs` 统一拥有，改形状前需明确 owner 与测试。

## 命令面（收束后）

命令以"核心 + support/legacy"分组，详见 `docs/COMMANDS.md`。重构中已移除大量重复别名（provider/credentials/session/env 各家族的冗余别名与未文档化解析别名），保留规范命令；命令清单与 `docs/COMMANDS.md` 的一致性由 `tests/mvp_contract.rs::command_docs_match_registry` 守护。

## 数据与产物

- 配置：`.deepcli/config.json`、`.deepcli/credentials/`（默认 gitignore）。
- 会话：`.deepcli/sessions/<id>/`（metadata/messages/tool calls/audit/diffs/backups）。
- 产品循环证据：`.deepcli/benchmarks/`、`.deepcli/baselines/`（本地，不提交）。

## 当前重构方向

按 `docs/ai/HARNESS_REFACTOR_PLAN.md`：命令 handler 已按领域拆分到 `src/commands/*`（阶段 3）；schema-id 去硬编码已完成（阶段 2）；命令面收束与重复删除进行中（阶段 6）；文档归并为总览 + 模块说明 + ADR（阶段 4）；docsync 检查扩展（阶段 5）；UI 收束为消费 projection（阶段 7）为后续方向。不可逆决策记录在 `docs/ADR/`。
