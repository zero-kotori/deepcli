# deepcli 当前对话上下文

本文是后续 agent 进入仓库时的当前上下文。以当前 worktree、`README.md`、`docs/ARCHITECTURE.md`、`docs/HARNESS.md`、`docs/COMMANDS.md`、`docs/CORE_FEATURES.md` 和 `docs/MODULES/` 为准。

## 当前产品状态

deepcli 已经是一个 local-first AI 编程代理 CLI，当前重点是维护真实产品路径的一致性，而不是继续沿用旧的开放式竞品对齐目标。

已落地的主要路径：

- 原生终端聊天和 one-shot 任务入口。
- DeepSeek-compatible Provider、Kimi 相关配置和统一 provider 接口。
- Agent Runtime、ContextManager、ToolRegistry、ToolExecutor、PermissionEngine；普通 Agent turn 统一走 tool-capable 流式 Provider 循环。
- 持久化会话、resume、fork、session inspect/search/diagnose。
- 文件、shell、Git、测试、环境、web、terminal、prompt、skill、子 Agent 工具。
- diff/review/verify/handoff/preflight/gate 交付链路。
- selftest/doctor/diagnose/support/logs/trace/privacy 本地诊断链路。
- scorecard/round/benchmark/recipes/opportunities 本地健康、证据和工作流报告。
- 稳定 JSON schema id、可执行 `nextActions` 和 `checklist[]` 输出约定。
- Host-owned 精确调用审批（高危双确认、单次消费）、canonical/symlink/DeepIgnore 文件边界、测试发现命令约束、shell child 凭据清洗、`run_shell`/`run_tests` 超时、真实工具 success 和批次 UI 汇总。

旧文档中的产品循环表述只作为历史命名背景存在。`recipes sota` 仍是兼容命令主题名，但不再代表一个未完成的产品要求。

## 当前架构事实

- `scripts/deepcli` 是本地 wrapper。
- `src/main.rs` 初始化进程并调用 CLI。
- `src/cli.rs` 负责参数解析、别名归一化、本地 one-shot 命令路由和交互入口选择。
- `src/runtime.rs` 负责 Agent loop、Provider turn、工具调用循环和会话观测。
- `src/context_manager.rs` 负责上下文预算、压缩和保留片段投影。
- `src/providers.rs` 负责 Provider 适配。
- `src/tools.rs` 与 `src/tools/*.rs` 负责 capability-only Provider schema、host-owned 调用摘要/digest、参数校验和执行。
- `src/permissions.rs` 负责权限模式、应用层 sandbox 策略和风险分级。
- `src/session.rs` 负责会话持久化，包括 digest-bound 审批及其单次消费状态。
- `src/commands.rs` 负责命令分发和跨模块 re-export，主体实现位于 `src/commands/*.rs`。
- `src/ui.rs`、`src/ui/native_terminal.rs`、`src/ui/resume_picker.rs` 负责当前原生终端 UI。

当前安全边界不能表述为 OS 强隔离：尚无 OS 级 shell/network sandbox 和完整 process-group cancellation。子 Agent allowed-tools、canonical read/write scope 与 host-owned depth 已由 runtime/executor capability 强制；带 scope 的广域工具 fail closed。`autoReviewer` 默认关闭，显式开启也不是可信执行边界。

## 文档权威关系

- README 只说明项目做了什么、怎么做、怎么快速运行和验证。
- `docs/ai/REQUIREMENTS.md` 记录当前范围、非目标、权限模型和验收标准。
- `docs/ai/TECHNICAL_PLAN.md` 记录当前调用链和实现方式，不再维护旧设计草案。
- `docs/FEATURES.md` 记录当前已落地功能。
- `docs/COMMANDS.md` 是命令分组、owner、legacy 策略的权威文档。
- `docs/CORE_FEATURES.md` 是核心功能和 JSON 契约入口。
- `docs/ARCHITECTURE.md`、`docs/HARNESS.md`、`docs/MODULES/*.md` 是模块边界和 docsync 规则入口。

## 工作规则

- 修改前先读相关代码和权威文档。
- 保持最小必要改动，不做无关重构。
- 行为改动必须连接真实 runtime/provider/tool/session 路径。
- 不提交 `.deepcli/benchmarks/`、`.deepcli/baselines/`、`.deepcli/exports/`、`.deepcli/support/`、credentials、logs、sessions 等本地产物。
- 提交身份使用 `zero-kotori <kotorizero8@gmail.com>`。

## 验证与提交

按改动范围选择验证：

- 文档-only：`git diff --check`，必要时运行相关 doc contract test。
- 命令文档或 registry：`cargo test command_docs_match_registry --test mvp_contract`。
- harness/module 文档：`cargo test architecture_harness_docs_cover_commands_and_modules --test mvp_contract`。
- 代码改动：运行相关单测，再按风险扩大到 `cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 隐私快速检查：`./scripts/deepcli privacy --no-history --json`。
- 提交前快速 gate：`./scripts/deepcli preflight --quick --json`。

完成一轮可安全提交时，先检查 diff 和隐私扫描结果，再提交。
