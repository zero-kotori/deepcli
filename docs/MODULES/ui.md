# UI 模块

## 职责

`src/ui.rs` 负责终端 UI：消息输入、对话记录渲染、监视标签页、快捷操作、resume 选择器、审批交互、运行安全的副命令分发，以及任务观测布局。

## 边界

- UI 应渲染领域状态并收集用户意图；它不应定义权威的命令、工具、权限或会话契约。
- 运行安全的命令分发必须与实际的处理器支持及命令文档保持一致。
- UI 投影应来自运行时、会话与命令报告模型，而不是解析终端文本。
- 高风险操作应预填或经由审批路由，而不是由含糊的 UI 点击触发。
- monitor 标签分为 core 与 advanced 两档（`MonitorTab::tier()`）：core 任务视图（Overview/Changes/Tools/Tests/Approvals）排在标签条前列，advanced/support 诊断视图（Result/Usage/Health/Library/Deliver/Environment/Trace）排在分隔符之后。标签顺序由 `MonitorTab::all()` 单一来源定义，`next()`/`previous()` 与渲染、点击命中都从该来源派生，避免多处各写一份顺序。

> Stage 7 现状：标签分档、core 优先排序与单一来源的渲染/点击命中已落地并由 `ui::tests::monitor_tab_*` 单测覆盖；进一步把 advanced 视图收进可切换的 advanced 面板属于改变实时 TUI 交互体验的改动，需要在本地 `./scripts/deepcli` 交互验证后再推进，本环境无法对交互体验做验收。

## 测试

- 针对性的 `ui::tests::*`，覆盖消息输入、监视标签页、快捷操作、运行安全守卫、resume 选择器、审批与渲染。
- `ui::tests::monitor_tab_*` 覆盖 core/advanced 分档、core 优先排序、`next()`/`previous()` 与排序来源一致，以及标签条分隔符与点击命中。
- 当 UI 消费稳定 JSON 或命令检查清单时的命令契约测试。
- `cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract` 用于文档同步覆盖。

## 文档同步

当监视标签页、运行安全行为、UI 投影职责归属或高风险交互规则发生变化时，更新本文件。当运行安全的公开行为发生变化时，更新 `docs/COMMANDS.md`。
