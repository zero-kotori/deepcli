# 工具模块

## 职责

`src/tools.rs` 负责工具执行，涵盖文件读写、打补丁、shell、Git、测试、网络搜索、终端启动、prompt/skill 辅助以及 subagent 派生。`src/tools/declarations.rs` 负责 `ToolDeclaration`、`ToolRegistry` 以及权限请求的构建。`src/tools/schema.rs` 负责用于构建 `ToolSpec` 的 provider 参数 schema。`src/tools/process.rs` 负责 shell 命令的输出模型与进程执行辅助。`src/tools/file.rs` 负责工作区路径解析、补丁规范化、写入保护、文本切片以及 unified diff 辅助。`src/tools/git.rs` 负责 Git 辅助校验与提交信息派生。`src/tools/environment.rs` 负责环境检查/设置模型、Docker/编译器就绪检测、设置动作以及环境报告格式化。`src/tools/test_discovery.rs` 负责项目测试命令发现与已发现测试的格式化。`src/tools/web.rs` 负责网络搜索响应的格式化。

## 边界

- 工具在执行写入、shell、Git、网络、Docker、终端或设置动作时，不得绕过 `src/permissions.rs`。
- 工具声明、参数 schema、权限面以及审计生命周期应保持为类型化声明契约的组成部分。
- 主要的工具执行路径应通过 `ToolDeclaration::permission_request` 配合 `ToolPermissionContext` 来评估权限；显式的文件系统辅助方法仅保留给文件操作及文件子操作使用。
- 命令处理器与运行时应通过 registry/executor 调用工具，而不是重复实现工具行为。
- 本地基准测试产物与支持包仍属于被忽略的工作区证据，不得提交。

## 测试

- `cargo test mvp_tool_registry_exposes_required_tools --test mvp_contract`
- `cargo test tool_declarations_own_provider_schema --test mvp_contract`
- `cargo test tool_declarations_build_permission_requests --test mvp_contract`
- 针对性的 `tools::tests::*`，覆盖路径安全、审批、打补丁、shell/测试执行、prompt/skill/subagent 辅助以及环境动作。
- 针对 `/test`、`/env`、`/git`、`/terminal` 及相关报告的命令 JSON 测试。

## 文档同步

当工具职责归属、权限面、参数契约或审计生命周期发生变化时，更新本文件。当某个以工具为后端的命令改变其公开行为时，更新 `docs/COMMANDS.md`。
