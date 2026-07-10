# deepcli 技术说明

本文描述当前实现方式和关键调用链。早期方案中的目录草案、fullscreen TUI、开放式竞品对齐目标和外部项目验收目标不再作为当前技术要求；真实架构以 `docs/ARCHITECTURE.md`、`docs/HARNESS.md` 和 `docs/MODULES/*.md` 为准。

## 入口与调用链

1. `scripts/deepcli` 作为本地 wrapper，负责构建/定位 Rust 二进制、传入工作区参数，并提供常用别名兼容。
2. `src/main.rs` 初始化 tracing，然后调用 `src/cli.rs`。
3. `src/cli.rs` 解析参数、归一化 provider/模式别名、拦截明显拼错的顶层命令，并决定是进入本地命令、one-shot Provider 调用、resume picker，还是原生终端聊天。
4. 本地命令进入 `src/commands.rs` 和 `src/commands/*.rs`，通过 `CommandRouter` 解析 slash command，输出文本或稳定 JSON。
5. 需要模型参与的任务进入 `src/runtime.rs`，由 `AgentRuntime` 准备上下文，以携带相应工具 schema 的统一流式路径调用 Provider、执行工具调用、记录会话事件并返回结果。

## Runtime 与数据流

`AgentRuntime` 不直接访问文件系统、shell、网络或 Git。所有外部动作都通过 `ToolRegistry`、`ToolExecutor` 和 `PermissionEngine`：

```text
user input
  -> cli/router
  -> AgentRuntime
  -> ContextManager + SessionStore
  -> ProviderClient stream events (text + completed tool calls)
  -> tool calls / ToolBatchCompleted
  -> ToolExecutor + PermissionEngine
  -> SessionStore audit/messages/tool records
  -> native terminal output or command JSON
```

上下文准备由 `src/context_manager.rs` 负责，包括上下文预算、压缩策略和保留片段投影。Provider 适配位于 `src/providers.rs`，目前覆盖 DeepSeek-compatible 和 Kimi 相关路径。会话持久化位于 `src/session.rs`。

## 命令层设计

命令层已经按 owner 拆到 `src/commands/*.rs`。`src/commands.rs` 只保留分发、跨模块 re-export 和共享入口：

- `registry.rs` 拥有公开命令 metadata、分组、running-safe 和 legacy successor。
- `parser.rs` 负责 slash command 解析。
- `scorecard_report.rs`、`round_report.rs`、`benchmark_*`、`recipes.rs` 和 `opportunities.rs` 负责本地健康、证据和工作流报告。
- `session_*`、`fork.rs`、`resume.rs`、`goal.rs` 和 `plan.rs` 负责会话、恢复、分支和规划状态。
- `delivery_*`、`preflight.rs`、`privacy.rs` 和 `git.rs` 负责交付、检查和受控 Git。

稳定 JSON schema id 统一由 `src/schema_ids.rs` 提供。公开 JSON 尽量包含 `report`、可执行 `nextActions` 和 `checklist[]`，便于终端、脚本和外部 UI 复用。

## 工具与权限

工具能力分布在 `src/tools.rs` 和 `src/tools/*.rs`。工具声明向 Provider 暴露名称和 capability-only 参数 schema，授权字段不属于模型输入；host 根据工具名与已解析参数构造权限请求，执行必须经过 `PermissionEngine`。

需要批准时，工具名与经 host 解析的有效参数 canonical JSON 生成 SHA-256 digest，审批记录保存脱敏摘要、确认计数和状态。普通决策确认一次，高危决策确认两次；只有完全匹配的 approved grant 能执行，并立即转为 consumed。`ToolExecution.success` 独立于 Rust 调用是否返回 `Ok`，用于表达非零 exit、测试失败和超时，并投影到 Provider tool result、生命周期和 UI。

文件路径会 canonicalize 工作区及目标的最近既存祖先，拒绝 symlink 逃逸；read/write/patch 目标还要经过 DeepIgnore。`run_tests` 的命令必须匹配工作区发现结果或其受限参数扩展，拒绝 shell 控制符、runner 配置覆盖和工作区外参数。shell helper 启动 child 前清除 Provider API key、token、secret、private/access key 等敏感环境变量；`run_shell` 与 `run_tests` 使用有界超时。`web_fetch` 对 DNS、redirect 和响应体实施 SSRF/容量边界；`git_commit` 使用批准 tree 的 `commit-tree`/`update-ref` CAS 路径。

子 Agent resume 从持久任务构建不可变 capability：allowed-tools 裁剪 Provider registry 并在 Executor 再校验，read/write scope 在 canonical 路径上检查，host 计算下一层 depth，嵌套 capability 只能收窄。带 scope 的广域 shell/Git/test 工具 fail closed。

权限策略的默认方向：

- 工作区内读操作可授权后执行。
- 写入、shell、Git、网络、Docker、终端和环境 setup 需要风险判断。
- `autoReviewer` 默认关闭；显式开启时只对校验后的测试/构建入口做确定性审批。
- 破坏性命令、系统写入、依赖安装和远程 Git 操作必须升级审批或二次确认。

这里的 sandbox 是应用层策略，不等同于 OS 级 shell 或 network 隔离。当前超时/取消没有覆盖完整 process group；显式开启 `autoReviewer` 也不提供仓库代码隔离。

## UI

当前 UI 是原生终端聊天，不再维护旧 fullscreen TUI。入口位于 `src/ui.rs`，主要实现位于：

- `src/ui/native_terminal.rs`：短 session header、`you`/`deepcli` 角色标签、bracketed paste 原生输入、终端控制序列清洗、低噪声流式输出、工具失败、审批提示和计划采访选项；Provider 请求指标与成功工具进度仅保留在 runtime/session 观测路径。
- `src/ui/resume_picker.rs`：文本 resume picker。

默认 `deepcli` 和 `deepcli repl` 都进入同一 native terminal 路径。

## 本地数据

- 配置：`.deepcli/config.json`
- 凭据：`.deepcli/credentials/` 或环境变量，输出必须脱敏
- 会话：`.deepcli/sessions/<id>/`
- benchmark/baseline 证据：`.deepcli/benchmarks/`、`.deepcli/baselines/`，只保留本地，不提交
- support/export 临时产物：`.deepcli/support/`、`.deepcli/exports/`，默认不提交

## 验证策略

按改动面选择最小验证：

- 文档：`git diff --check`
- 命令文档/registry：`cargo test command_docs_match_registry --test mvp_contract`
- harness/module 文档：`cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract`
- runtime/session/tool 改动：运行对应模块单测和受影响命令契约测试
- 提交前快速检查：`cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`、`./scripts/deepcli preflight --quick --json`
- 隐私检查：`./scripts/deepcli privacy --no-history --json`；发布或推送前运行完整 `preflight`
