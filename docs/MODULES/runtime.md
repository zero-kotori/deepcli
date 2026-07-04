# 运行时模块

## 职责

`src/runtime.rs` 负责 agent 循环、provider 轮次生命周期、工具调用循环、上下文组装、计划状态更新，以及供状态与 UI 界面使用的会话观测。

## 边界

- 运行时负责编排 provider 与工具工作；它不应沦为命令解析器或 UI 渲染器。
- 工具执行必须经过 `ToolExecutor` 与权限检查。
- 会话状态变更必须经过会话 API。
- provider 特定的请求与流式解析归属于 `src/providers.rs`。
- agent loop 不设置固定 provider/tool 轮次上限；结束条件来自模型最终回答、用户停止、权限等待、provider timeout 或真实错误。
- context/verification 工具预算默认不启用；只有显式设置 `DEEPCLI_MAX_CONTEXT_TOOL_CALLS` 或 `DEEPCLI_MAX_VERIFICATION_TOOL_CALLS` 时，运行时才会跳过对应工具并把恢复提示反馈给模型继续处理。
- 上下文压缩行为不属于当前 harness 重构的范围，除非另行编写专门的计划。

## 测试

- 针对性的 `runtime::tests::*`，覆盖 provider 轮次、工具循环、会话观测与计划行为。
- 仅当运行时状态被外部投影时，才编写命令或 UI 测试。
- `cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract` 用于文档同步覆盖。

## 文档同步

当运行时职责归属、事件来源、观测结构或上下文边界发生变化时，更新本文件与 `docs/HARNESS.md`。当运行时变更改变了用户可见的 JSON 报告时，更新稳定的命令文档。
