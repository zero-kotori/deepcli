# deepcli 功能介绍

> 持续更新中：本文档记录当前已经落地并可验收的主要功能。随着产品继续向 SOTA 编程代理 CLI 演进，本功能清单会同步更新。

## 产品定位

deepcli 是一个 local-first 的 AI 编程代理 CLI。它以当前工作目录为中心，提供 TUI 交互、Provider/模型切换、会话恢复、工具调用、安全审批、环境准备、测试验收、诊断支持包和 shell 集成等能力。

核心目标是让用户可以在一个命令行工具里完成：

- 启动和恢复 AI 编程任务。
- 配置 DeepSeek/Kimi 等 Provider 与模型。
- 让 Agent 读取项目、修改代码、执行工具、运行测试。
- 对环境、凭据、会话、日志和命令入口进行本地自检。
- 在交付前生成验收报告、严格 gate、handoff 或 PR 描述。

## 启动与入口

deepcli 提供脚本入口和 Rust 二进制入口：

- `deepcli`：默认进入 TUI。
- `deepcli tui`：显式进入 TUI。
- `deepcli repl`：进入兼容的行式 REPL。
- `deepcli ask <prompt>`：一次性任务。
- `deepcli stream <prompt>`：流式一次性任务。
- `deepcli deepseek ...`：使用 DeepSeek provider 预设。
- `deepcli kimi ...`：使用 Kimi provider 预设。
- `deepcli recipes [topic]`：查看任务型工作流命令清单。
- `deepcli scorecard [--json]`：查看产品能力覆盖、SOTA 差距和 benchmark 证据。
- `deepcli benchmark presets|run|record|status|summary|list|show [--json]`：发现推荐 workload、执行、记录、评估证据质量、汇总、列出和查看本地 benchmark 证据 artifact。

启动 wrapper 会自动补充当前工作目录、配置路径和 yes 授权默认值，同时保留显式参数。

## TUI 与交互体验

TUI 面向实际编码任务，而不是简单聊天框：

- message box 支持编辑、粘贴、多行输入和历史输入。
- slash command palette 支持过滤、选择和补全。
- 会话消息会从持久化记录恢复。
- Agent 运行中仍可执行本地安全命令，例如 `/status`、`/usage`、`/trace`、`/logs`、`/privacy`、`/recipes`、`/scorecard`、`/benchmark`、`/selftest`、`/preflight`、`/completion`、`/session`、`/approval`、`/stop` 和 `/quit`。
- 工具调用默认以可扫描的任务观察面板呈现，并支持查看工具详情。

## 会话管理

会话是 deepcli 的核心状态单元：

- `deepcli resume` 打开会话选择器。
- `deepcli resume <session_id>` 恢复指定会话。
- `deepcli sessions --all --limit 20` 查看历史。
- `deepcli history` 是历史列表快捷入口。
- `/rename` 可重命名当前或指定会话。
- `/session search` 可按标题、摘要、消息、工具调用、测试、diff 等搜索历史。
- `/cleanup sessions` 可预览或删除空的一次性会话。

## Provider、模型与凭据

deepcli 当前面向 DeepSeek-compatible providers，并内置 DeepSeek/Kimi 相关入口：

- `deepcli model show|list`
- `deepcli use <provider> [model]`
- `deepcli switch <provider> [model]`
- `deepcli provider [provider] [model]`
- `deepcli providers --json`

凭据相关命令都在本地执行，不需要先创建会话或调用 provider：

- `deepcli credentials status [provider] --json`
- `deepcli login <provider> --stdin --force`
- `deepcli auth|apikey|key`
- `deepcli logout <provider>`
- `deepcli credentials template <provider>`
- `deepcli credentials import-env <provider>`

输出会脱敏，不打印明文 API key。

## 本地健康检查与安装验收

deepcli 内置多层本地检查能力：

- `deepcli selftest --json`：产品自身安装与命令面自检。
- `deepcli doctor --quick --json`：工作区健康检查。
- `deepcli doctor shell --json`：shell 安装健康检查。
- `deepcli health --json`：快捷健康检查。
- `deepcli version --json` / `deepcli about --json`：版本与支持元数据。

`selftest` 和 `doctor` 会读取项目配置中的 `project.gitIdentity`，对比当前 Git 仓库有效的 `user.name` / `user.email`，在提交前提示错误作者身份，并给出可复制的 `git config` 修复命令。非 Git 目录只报告 `no_git`，不会读取全局 Git 身份。

`doctor shell` 会检查：

- `deepcli` 是否在 PATH。
- PATH 中的 `deepcli` 是否解析到当前 workspace 的 `scripts/deepcli` 或 `target/debug/deepcli`。
- 旧命令名是否残留。
- bash/zsh/fish completion 是否缺失、过期或已是最新。

## Shell 集成

补全能力覆盖顶层命令、provider 快捷入口和常用参数：

- `deepcli completion zsh`
- `deepcli completion bash`
- `deepcli completion fish`
- `deepcli completion json`
- `deepcli completion status zsh --json`
- `deepcli completion install zsh --force`

`install` 默认 dry-run，只有显式 `--force` 才写入用户 HOME 下的 allowlisted completion 文件。

## 环境与工具链

deepcli 可以检查、规划和准备本地任务环境：

- `deepcli env check docker --json`
- `deepcli env plan compiler --smoke --json`
- `deepcli setup docker --smoke`
- `deepcli install compiler --smoke`
- `deepcli env test compiler --json`

环境 setup/test 走权限和工具审计路径；只读 check/plan 可作为快速预检。

## 测试、验收与交付

deepcli 不只负责生成代码，也负责形成交付证据：

- `deepcli recipes release --json`
- `deepcli scorecard --json`
- `deepcli benchmark presets --json`
- `deepcli benchmark status --json`
- `deepcli benchmark summary --json`
- `deepcli test discover --json`
- `deepcli test run --json -- cargo test`
- `deepcli accept --json`
- `deepcli gate --json`
- `deepcli verify --json`
- `deepcli handoff --pr`
- `deepcli preflight --json`

验收报告会聚合 Git 状态、diff、review 风险、测试证据、环境证据、失败工具、待审批和会话信号。无当前会话的一次性 `accept` / `gate` 会优先使用本次 workspace 测试证据，避免历史 session 的旧失败污染最终验收。

`preflight` / `release-check` 是提交/推送前的一键本地检查入口，串联 `cargo fmt --check`、`git diff --check`、`cargo clippy --all-targets -- -D warnings`、`selftest`、`doctor --quick`、`privacy --fail-on-findings` 和 `gate --json`，并输出稳定 JSON 报告；`--dry-run` 只预览检查清单，`--quick` 跳过较慢的 clippy/gate。

`recipes` / `playbook` 是任务型工作流目录，按 start、code、debug、release、support、environment、shell 等主题输出可复制命令和稳定 `deepcli.recipes.v1` JSON，适合 TUI、外部 UI 或团队脚本引导用户选择下一步；该命令本地只读，不创建 session、不调用 Provider。

`scorecard` 是产品能力评分和 SOTA 差距入口，按命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据给出 0-100 分、tier、gaps、next actions 和稳定 `deepcli.scorecard.v1` JSON；`--fail-below` 可作为本地产品门禁，命令不创建 session、不调用 Provider。`benchmark` 保留无子命令和 scorecard flags 的兼容行为，并增加 `presets/run/record/status/summary/list/show`：`presets` 列出 cargo-test、preflight-quick、selftest、scorecard 和 smoke 等推荐 workload，`run --preset <name>` 显式执行对应本地命令、采集 exit code、耗时和输出摘要并写入 `.deepcli/benchmarks/*.json`，`record` 只记录声明证据，`status` 输出稳定 `deepcli.benchmark.status.v1` 并把证据判定为 missing、weak、failing、stale 或 ready，`summary` 聚合历史 artifact 的通过率、失败数、耗时范围和最新 artifact，`list/show` 用于本地验收和持续产品循环。

## 诊断、日志与支持包

当出现慢响应、凭据、环境、工具或测试问题时，可以先本地诊断：

- `deepcli status --json`
- `deepcli usage --json`
- `deepcli trace --limit 30`
- `deepcli logs --limit 80`
- `deepcli privacy --json`
- `deepcli diagnose --json`
- `deepcli support .deepcli/support/latest`

支持包会脱敏，便于提交 issue 或内部工单。

## Prompt、Skill 与子 Agent

deepcli 提供可扩展的任务能力库：

- `deepcli prompt list|get|render`
- `deepcli skill list|run`
- `deepcli agent list|show`

这些命令支持 JSON 输出，可用于 TUI 面板、外部 UI 或脚本化集成。

## 安全与权限

deepcli 默认强调本地安全边界：

- 工作区写入和危险 shell 命令走权限策略。
- Docker、安装包、系统写入等操作需要更严格审批。
- 凭据、日志、trace、support bundle 输出会脱敏。
- `deepcli privacy` 可在开源或共享前扫描 git history、提交邮箱、本机绝对路径、敏感路径和疑似密钥，并支持 JSON artifact 与 `--fail-on-findings`。
- `privacy.allowedEmails` / `privacy.allowedEmailDomains` 可把确认公开或允许的邮箱从 `/privacy` findings 中折叠到 suppressed findings，`privacy.allowedCommitEmails` / `privacy.allowedCommitDomains` 可只允许提交元数据邮箱，降低开源前检查噪声。
- `privacy.allowedUserPaths` 可把确认可接受的历史本机用户路径折叠到 suppressed findings，避免迁移旧路径长期阻断发布检查。
- `project.gitIdentity` 可把仓库预期提交身份写入 `.deepcli/config.json`，由 `doctor` / `selftest` 作为正式健康检查项持续验证。
- 只读 one-shot 命令不应创建空会话或污染项目授权状态。

## 常用验收命令

开发或发版前建议运行：

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
./scripts/deepcli selftest --json
./scripts/deepcli doctor shell --json
./scripts/deepcli recipes release --json
./scripts/deepcli scorecard --json
./scripts/deepcli benchmark presets --json
./scripts/deepcli benchmark list --json
./scripts/deepcli benchmark status --json
./scripts/deepcli benchmark summary --json
./scripts/deepcli preflight --json
./scripts/deepcli release-check --dry-run
```

## 后续方向

持续改进方向包括：

- 更强的 TUI 信息架构和任务观察面板。
- 更完整的自动环境准备与 smoke test。
- 更智能的 session 恢复、搜索和交接。
- 更系统的 provider 延迟、上下文压缩和工具失败诊断。
- 更正式的端到端 benchmark workload 执行、横向模型/工具对比和趋势分析。
- 更接近 SOTA 编程代理的端到端任务闭环。
