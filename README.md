# deepcli

deepcli 是一个 local-first 的 AI 编程代理 CLI。它把原生终端聊天、Provider/模型切换、会话恢复、受控工具调用、权限审批、测试验收、诊断和交付报告放在同一个本地命令行工作流里。

## 我们做了什么

- 做了一个 Rust CLI 和 `scripts/deepcli` wrapper，默认在当前工作区启动原生终端聊天，也支持 `ask`/`stream` 一次性任务。
- 接入 DeepSeek-compatible Provider，并保留 Kimi 与其它兼容 Provider 的配置扩展点。
- 做了持久化会话系统：消息、工具调用、审计、plan、goal、diff、backup、审批和旁路问题都会落到 `.deepcli/sessions/`，可以 resume、搜索、诊断和 fork。
- 做了统一工具层：文件、shell、Git、测试、环境、web、terminal、prompt、skill、子 Agent 等能力通过声明、参数校验和权限层执行。
- 做了本地安全边界：工作区授权、ignore/隐私过滤、sandbox、危险命令识别、审批队列、凭据脱敏和隐私扫描。
- 做了交付与验证闭环：`test`、`diff`、`review`、`verify`、`handoff`、`preflight`、`gate`、`scorecard`、`round`、`benchmark` 都输出可脚本消费的 JSON 和可执行 next actions。

## 怎么做的

请求从 `scripts/deepcli` 进入 `src/cli.rs`。CLI 会先归一化 provider/模式别名，识别本地 one-shot 命令；能本地处理的命令直接走 `src/commands/*`，需要模型参与的任务才创建或恢复 `AgentRuntime`。

`src/runtime.rs` 负责 Agent 主循环：准备上下文、调用 Provider、执行工具调用、记录会话事件并把结果返回 UI。Agent 不直接访问文件系统、shell、网络或 Git，所有动作都经 `src/tools/*` 和 `src/permissions.rs`。

`src/session.rs` 是持久化状态边界。`src/context_manager.rs` 负责上下文预算和压缩。`src/schema_ids.rs` 拥有稳定 JSON schema 标识符。命令、模块和架构边界分别由 `docs/COMMANDS.md`、`docs/MODULES/`、`docs/ARCHITECTURE.md` 维护。

## 快速开始

```bash
cargo build
./scripts/deepcli

./scripts/deepcli ask "检查这个项目的测试入口"
./scripts/deepcli selftest --json
./scripts/deepcli doctor --quick --json
./scripts/deepcli credentials status --json
./scripts/deepcli model list --json
./scripts/deepcli resume --dry-run --json
./scripts/deepcli preflight --quick --json
```

## 常用验证

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
./scripts/deepcli selftest --json
./scripts/deepcli doctor --quick --json
./scripts/deepcli privacy --no-history --json
```

完整提交前可运行：

```bash
./scripts/deepcli preflight --json
```

## 文档

- [核心功能契约](docs/CORE_FEATURES.md)
- [命令分组](docs/COMMANDS.md)
- [架构](docs/ARCHITECTURE.md)
- [Harness](docs/HARNESS.md)
- [模块说明](docs/MODULES/)
- [功能介绍](docs/FEATURES.md)
- [当前范围](docs/ai/REQUIREMENTS.md)
- [技术说明](docs/ai/TECHNICAL_PLAN.md)
