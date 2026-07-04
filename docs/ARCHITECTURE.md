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
       └─ src/ui.rs + src/ui/*    TUI：入口编排、外置测试、输入状态机、worker drain、runtime lifecycle、任务观察面板、dashboard、session projection、running-safe owner 与渲染，monitor projection owner
```

支撑模块：`src/config.rs`(有效配置、serde 默认与凭据引用)、`src/context_manager.rs`(ContextManager、ContextCompactionOptions、ContextPreparation、prepare、microcompact_tool_outputs、compact_messages_for_provider、provider_messages_to_retained_segment、message_groups_omitted_after_compaction)、`src/workspace.rs`(工作区授权与上下文过滤)、`src/privacy.rs`(脱敏)、`src/prompts.rs`/`src/skills.rs`(本地库)、`src/agents.rs`(子 Agent task、lifecycle、事件日志和恢复元数据)、`src/schema_ids.rs`(稳定 JSON schema 标识符的所有权 registry)。

## 关键边界原则

- Agent 不直接碰文件系统/shell/网络/Git，一切经工具系统 + 权限引擎。
- 命令层可解析输入并构建报告，但持久领域行为应落在有所有权的模块或 registry。
- UI 渲染状态、收集输入，不作为命令/工具/会话/权限行为的真理来源（收束方向：UI 消费领域 projection）。
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

按 `docs/ai/HARNESS_REFACTOR_PLAN.md`：命令 handler 已按领域拆分到 `src/commands/*`（阶段 3），大型命令契约测试已外置到 `src/commands/tests.rs`，Git identity 报告已拆到 `src/commands/git_identity.rs`，无状态共享 helper 已拆到 `src/commands/shared.rs`，会话共享 helper 已拆到 `src/commands/session_helpers.rs`，环境 nextActions helper 已拆到 `src/commands/environment_actions.rs`，command group/legacy policy projection 已拆到 `src/commands/command_policy.rs`，action checklist/filter/label 投影已拆到 `src/commands/action_checklist.rs`，scorecard report builder、scorecard category projection、scorecard summary JSON 与 scorecard text/JSON output 已拆到 `src/commands/scorecard_report.rs`，scorecard opportunities、opportunity projection、recommended opportunity 与 opportunity counts 已拆到 `src/commands/scorecard_opportunities.rs`，benchmark dispatch、scorecard-compatible benchmark args 与 benchmark gate dispatch 已拆到 `src/commands/benchmark_dispatch.rs`，round benchmark gate projection、benchmark trend gate、round benchmark status projection 与 freshness suffix 已拆到 `src/commands/round_benchmark_gates.rs`，round goal status、goal readiness projection、goalStatus JSON 与 goal_readiness gate support 已拆到 `src/commands/round_goal_status.rs`，round report builder、round text/JSON output、round summary JSON 与 round benchmark suite wrapper 已拆到 `src/commands/round_report.rs`，benchmark artifact list/show/cleanup 与 artifact projection 已拆到 `src/commands/benchmark_artifacts.rs`，benchmark presets catalog 与 required/default preset projection 已拆到 `src/commands/benchmark_presets.rs`，benchmark run/record/run-suite execution artifact、shell timeout、suite schema 与 artifact path slug 已拆到 `src/commands/benchmark_runs.rs`，benchmark baseline-template/inventory/compare-ready projection 已拆到 `src/commands/benchmark_baselines.rs`，benchmark summary/trends/compare history projection 与 trend gate 状态已拆到 `src/commands/benchmark_history.rs`，benchmark status handler、status schema、freshness projection 与 required preset 覆盖状态已拆到 `src/commands/benchmark_status.rs`，delivery diff projection、path scope filtering、session diff fallback 与 diff stat/name-only projection 已拆到 `src/commands/delivery_diff.rs`，delivery report builder、verification report projection、handoff report projection 与 delivery report JSON 已拆到 `src/commands/delivery_reports.rs`，session catalog owner、session list/search projection、session catalog JSON 与 prune-empty report 已拆到 `src/commands/session_catalog.rs`，session restore-backup owner、restore-backup dry-run 与 restore preview JSON 已拆到 `src/commands/session_restore.rs`，session inspect owner、session record projection、session inspect JSON 与 tools/tests/diffs/backups projection 已拆到 `src/commands/session_inspect.rs`，session recovery owner、session next/diagnose projection 与 next-action signals 已拆到 `src/commands/session_recovery.rs`；schema-id 去硬编码已完成（阶段 2）；公开命令分组、running-safe 标记、legacy successor/policy、parser 兼容 alias 与 completion-only aliases 已收束到显式 command metadata registry，并由契约测试守护；命令面删除/降级审计已在 `docs/COMMANDS.md` 落地并由契约测试约束（阶段 6）；文档归并为总览 + 模块说明 + ADR（阶段 4）；docsync 检查扩展（阶段 5）；UI 大型单测已外置到 `src/ui/tests.rs`，UI running-safe 提示已消费 command registry projection，chat view 的主聊天布局、transcript 渲染和输入光标定位已迁入 `src/ui/chat_view.rs`，chat history 行模型、session/runtime 消息转换和长历史截断已迁入 `src/ui/chat_history.rs`，active session 引用、header 状态、SessionMonitor fallback、plan 摘要和 workspace fallback 已迁入 `src/ui/session_projection.rs`，message box 输入 buffer/cursor/history 编辑状态机与 prompt 输入 helper 已迁入 `src/ui/message_box.rs`，通用 UI 文本截断、短 ID、usage/environment 格式化和最新 action 输出摘要已迁入 `src/ui/text.rs`，worker progress/done channel drain、`WorkerDone` envelope、工具日志写入和运行结果写回已迁入 `src/ui/worker.rs`，运行中任务停止、session paused 写回和交互 runtime rebuild 已迁入 `src/ui/runtime_lifecycle.rs`，非交互 dashboard snapshot 与渲染已迁入 `src/ui/dashboard.rs`，slash command palette 查询、running-safe 排序、键鼠选择、点击命中、完成输入和渲染已迁入 `src/ui/command_palette.rs`，credential prompt 的 `/credentials set` 解析、隐藏输入框、保存 API key 和隐藏光标计算已迁入 `src/ui/credential_prompt.rs`，运行中本地命令解析、read-only/write-output guard 与状态/BTW/Terminal/Git 旁路处理已迁入 `src/ui/running_commands.rs`，输入提交、空闲本地 TUI 命令和 resume 结果应用已迁入 `src/ui/input_submission.rs`，resume picker 状态、过滤、独立选择循环、键鼠处理、列表/预览布局和预览文本已迁入 `src/ui/resume_picker.rs`，Approvals tab 的审批/旁路问题选择、鼠标命中、批准/拒绝、BTW 回答提示和 session/runtime 写回已迁入 `src/ui/approvals.rs`，core monitor tabs 已补齐 Session/Context 并消费 `SessionMonitor` projection，tab order/label/tier 已收束到 `src/ui/monitor.rs` 的 `MonitorTabMetadata` projection，静态 monitor quick actions 已收束到同一 owner 的 `MonitorTabQuickActions` projection，Deliver/Environment 的 monitor-only dynamic quick actions 与 Usage/Deliver/Tests/Session/Context/Environment/Approvals 等 SessionMonitor-only formatter 已迁入同一 monitor owner，Changes workspace/session diff projection 已迁入 `src/ui/monitor_changes.rs`，Tools tool-log projection 已迁入 `src/ui/monitor_tools.rs`，Health workspace/config projection 已迁入 `src/ui/monitor_health.rs`，Library workspace/library projection 已迁入 `src/ui/monitor_library.rs`，Result/Trace output projection 已迁入 `src/ui/monitor_output.rs`，task monitor shell 已迁入 `src/ui/monitor_shell.rs`，真实 pty smoke gate 已新增为 `scripts/tui-smoke`，UI entrypoint final orchestration boundary 已由契约锁定，最终终端观感仍需收尾验证。不可逆决策记录在 `docs/ADR/`。

Delivery review heuristic、review risk detection 与 sensitive/dangerous/panic-prone finding projection 已拆到 `src/commands/delivery_review.rs`，`src/commands/delivery.rs` 只保留 `/diff` 与 `/review` 编排和 owner 委派。

UI transcript/result 键盘与鼠标滚动状态机已拆到 `src/ui/scrolling.rs`，`src/ui.rs` 只在键鼠事件分发时调用该 owner。

UI monitor quick action 选择、edit-before-run、提交和点击激活已拆到 `src/ui/quick_actions.rs`，`src/ui.rs` 只保留键鼠事件分发调用。

UI paste event 路由和换行归一化已拆到 `src/ui/paste.rs`，credential prompt、BTW answer prompt、resume filter、主输入框和 Tools detail 预填复用同一 owner。

UI 矩形命中和面板内容行命中 helper 已拆到 `src/ui/geometry.rs`，鼠标分发和各交互 owner 复用同一几何判断。

UI entrypoint final orchestration boundary 已锁定在 `src/ui.rs`：该入口只保留 `TuiState`、`run_basic_repl`、`run_tui`、`run_tui_loop`、`handle_tui_mouse`、`handle_tools_scroll_mouse`、`handle_tui_key` 和 `cycle_monitor_tab`，其余业务投影、输入状态机、quick action、paste、geometry、scrolling、worker drain 与 monitor render 均由 `src/ui/*` owner 承担。

Delivery verify/handoff owner、verify/handoff option parser、test/env execution helper、verification session selection 和 verification test run persistence 已拆到 `src/commands/delivery_verify.rs`，`src/commands/delivery.rs` 现在只保留 `/diff` 与 `/review` 编排和 diff/review owner 委派。

Session export parser、export path safety 与 session export JSON 写出已拆到 `src/commands/session_export.rs`，`src/commands/session.rs` 只保留 `/session export` 的分发委派。

Session rename parser、current-session rename 与 title update 已拆到 `src/commands/session_rename.rs`，`src/commands/session.rs` 只保留 `/session rename` 的分发委派。

可恢复会话筛选、low-information clarification 过滤、thin completed chat 过滤和 workspace resumable fallback 已拆到 `src/commands/session_resumable.rs`，供 `/resume`、`/fork`、selftest 与 TUI resume picker 复用。

Session selection owner、`SessionFallbackKind`、inspection fallback、scoped list/action parser、queue action parser、approval/BTW cross-session lookup、session note prefix 与 `short_id` 投影已拆到 `src/commands/session_selection.rs`，`src/commands/session.rs` 现在只保留 `/session` 主分发和 restore-backup running-safe 入口委派。
