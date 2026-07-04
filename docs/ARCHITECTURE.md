# deepcli 架构

本文描述 deepcli 当前的真实架构与分层。模块所有权与边界细节见 `docs/HARNESS.md` 和 `docs/MODULES/*.md`；命令面见 `docs/COMMANDS.md`；核心功能契约见 `docs/CORE_FEATURES.md`。

## 分层

deepcli 是一个 Rust CLI（`src/main.rs` 二进制 + `src/lib.rs` 库），外层是 `scripts/deepcli` 启动 wrapper。请求自上而下流经：

```
scripts/deepcli (wrapper)         顶层命令/别名 → slash，自动构建二进制、注入 -C/--yes/凭据
  └─ src/cli.rs                   参数解析、provider/模式归一化、one-shot 路由、交互入口选择
       ├─ src/commands/*          slash 命令解析(parser)、分发、各命令 handler、稳定 JSON 报告
       ├─ src/runtime.rs          Agent loop、provider turn、工具调用循环、会话观测
       ├─ src/context_manager.rs  ContextManager、上下文预算、micro/full/tail compaction、保留片段投影
       │    ├─ src/providers.rs   Provider 适配(DeepSeek/Kimi)、流式、tool call、usage、重试、代理
       │    ├─ src/tools/*        工具声明与执行(文件/shell/git/test/env/web/...)，经权限层
       │    └─ src/permissions.rs 权限模式、sandbox、风险分级、审批
       ├─ src/session.rs          会话持久化(消息/工具/审计/plan/goal/diff/backup/审批/旁路问题)
       └─ src/ui.rs + src/ui/*    原生终端 UI：native terminal 聊天入口、输入编辑、流式输出、工具进度折叠、plan 采访文本选项和文本 resume picker
```

支撑模块：`src/config.rs`(有效配置、serde 默认与凭据引用)、`src/context_manager.rs`(ContextManager、ContextCompactionOptions、ContextPreparation、prepare、microcompact_tool_outputs、compact_messages_for_provider、provider_messages_to_retained_segment、message_groups_omitted_after_compaction)、`src/workspace.rs`(工作区授权与上下文过滤)、`src/privacy.rs`(脱敏)、`src/prompts.rs`/`src/skills.rs`(本地库)、`src/agents.rs`(子 Agent task、lifecycle、事件日志和恢复元数据)、`src/schema_ids.rs`(稳定 JSON schema 标识符的所有权 registry)。

## 关键边界原则

- Agent 不直接碰文件系统/shell/网络/Git，一切经工具系统 + 权限引擎。
- 命令层可解析输入并构建报告，但持久领域行为应落在有所有权的模块或 registry。
- UI 渲染状态、收集输入，不作为命令/工具/会话/权限行为的真理来源；当前只保留原生终端聊天路径。
- runtime 编排 provider turn 与工具循环，不拼 UI 文案、不绕过 `SessionStore`；上下文预算、压缩、provider-assisted summary、tail compact 和 retained boundary 投影由 `src/context_manager.rs` 拥有。
- 工具的写入/shell/Git/网络/Docker/终端/setup 操作必须经权限决策。
- 稳定 JSON schema 由 `src/schema_ids.rs` 统一拥有，改形状前需明确 owner 与测试。

## 命令面（收束后）

命令以"核心 + support/legacy"分组，详见 `docs/COMMANDS.md`。重构中已移除大量重复别名（provider/credentials/session/env 各家族的冗余别名与未文档化解析别名），保留规范命令；命令清单与 `docs/COMMANDS.md` 的一致性由 `tests/mvp_contract.rs::command_docs_match_registry` 守护。`deepcli completion json` 输出 `groups[]` 与 `legacyCommands[]`，由 `src/commands/command_policy.rs` 从 registry/policy metadata 投影，覆盖 slash legacy 命令和 completion-only legacy alias，供外部 UI 降级展示 legacy 入口并指向替代命令。

## 数据与产物

- 配置：`.deepcli/config.json`、`.deepcli/credentials/`（默认 gitignore）。
- 会话：`.deepcli/sessions/<id>/`（metadata/messages/tool calls/audit/diffs/backups）。
- 产品循环证据：`.deepcli/benchmarks/`、`.deepcli/baselines/`（本地，不提交）。

## 当前重构方向

当前命令层、runtime、tools、session 和权限模块继续按 owner 拆分维护；UI 已从旧 fullscreen TUI 收敛为原生终端聊天路径，只保留 `src/ui.rs`、`src/ui/native_terminal.rs` 和 `src/ui/resume_picker.rs`。历史 TUI/Ratatui owner、monitor tabs、dialogs、dashboard、command palette 和 running TUI side-command 文件已删除，默认 `deepcli` 与 `deepcli repl` 都进入 native terminal。

Delivery review heuristic、review risk detection 与 sensitive/dangerous/panic-prone finding projection 已拆到 `src/commands/delivery_review.rs`，`src/commands/delivery.rs` 只保留 `/diff` 与 `/review` 编排和 owner 委派。

UI transcript/result 键盘与鼠标滚动状态机已拆到 `src/ui/scrolling.rs`，`src/ui.rs` 只在键鼠事件分发时调用该 owner。

UI monitor quick action 选择、edit-before-run、提交和点击激活已拆到 `src/ui/quick_actions.rs`，`src/ui.rs` 只保留键鼠事件分发调用。

UI paste event 路由和换行归一化已拆到 `src/ui/paste.rs`，credential prompt、BTW answer prompt、resume filter、主输入框和 Tools detail 预填复用同一 owner。

UI 矩形命中和面板内容行命中 helper 已拆到 `src/ui/geometry.rs`，鼠标分发和各交互 owner 复用同一几何判断。

UI entrypoint 已收敛为原生终端薄入口：`src/ui.rs` 只注册 `native_terminal` 和 `resume_picker`，并导出 `run_basic_repl` 与 resume picker API。

Delivery verify/handoff owner、verify/handoff option parser、test/env execution helper、verification session selection 和 verification test run persistence 已拆到 `src/commands/delivery_verify.rs`，`src/commands/delivery.rs` 现在只保留 `/diff` 与 `/review` 编排和 diff/review owner 委派。

Session export parser、export path safety 与 session export JSON 写出已拆到 `src/commands/session_export.rs`，`src/commands/session.rs` 只保留 `/session export` 的分发委派。

Session rename parser、current-session rename 与 title update 已拆到 `src/commands/session_rename.rs`，`src/commands/session.rs` 只保留 `/session rename` 的分发委派。

可恢复会话筛选、low-information clarification 过滤、thin completed chat 过滤和 workspace resumable fallback 已拆到 `src/commands/session_resumable.rs`，供 `/resume`、`/fork`、selftest 与 native resume picker 复用。

Session selection owner、`SessionFallbackKind`、inspection fallback、scoped list/action parser、queue action parser、approval/BTW cross-session lookup、session note prefix 与 `short_id` 投影已拆到 `src/commands/session_selection.rs`，`src/commands/session.rs` 现在只保留 `/session` 主分发和 restore-backup running-safe 入口委派。
