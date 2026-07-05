# deepcli 已实现功能回顾

本文是当前实现概览，不作为未来需求清单。权威功能说明见 `docs/FEATURES.md`，命令分组见 `docs/COMMANDS.md`，架构边界见 `docs/ARCHITECTURE.md` 与 `docs/HARNESS.md`。

## 已实现的产品骨架

- Rust CLI 与 `scripts/deepcli` wrapper。
- 原生终端聊天、one-shot `ask`/`stream`、provider 前缀和帮助/补全入口。
- DeepSeek-compatible Provider、Kimi 配置路径和统一 provider 适配接口。
- Agent Runtime、上下文管理、工具调用循环和会话观测。
- 工具声明与执行：文件、shell、Git、测试、环境、web、terminal、prompt、skill、子 Agent。
- 权限层：工作区授权、sandbox、风险分级、审批队列和敏感输出脱敏。
- 会话系统：resume、fork、search、diagnose、diff/test/tool/backup inspection。
- 验收交付：test、diff、review、verify、handoff、preflight、gate。
- 本地诊断：selftest、doctor、diagnose、support、logs、trace、privacy。
- 本地健康和证据报告：scorecard、round、benchmark、recipes、opportunities。

## 当前实现约定

- 公开 JSON 输出尽量包含 `schema`、`report`、可执行 `nextActions` 和 `checklist[]`。
- 命令分组和 legacy 策略由 `src/commands/registry.rs`、`src/commands/command_policy.rs` 和 `docs/COMMANDS.md` 共同维护。
- schema id 由 `src/schema_ids.rs` 统一拥有。
- benchmark、baseline、support、export、session、log、credential 等本地产物不提交。
- `recipes sota` 是历史兼容主题名，表示当前产品证据/benchmark 工作流，不再作为开放式产品目标。

## 建议验证入口

```bash
cargo fmt --check
cargo test
git diff --check
./scripts/deepcli selftest --json
./scripts/deepcli doctor --quick --json
./scripts/deepcli privacy --no-history --json
./scripts/deepcli preflight --quick --json
```

按具体改动面补充 `docs/HARNESS.md` 中列出的契约测试。
