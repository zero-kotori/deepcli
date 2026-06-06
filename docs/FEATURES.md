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
- `deepcli goal [objective...]`：为当前会话写入长期目标契约和验收停止条件。
- `deepcli plan <rough requirement>`：围绕不成熟需求生成澄清问题、推荐选项和需求草稿。
- `deepcli fork [session_id|--current]`：复制已持久化会话上下文，并可打开新终端恢复到副本。
- `deepcli scorecard [--json]`：查看产品能力覆盖、SOTA 差距和 benchmark 证据。
- `deepcli round [--json] [--fail-on-gaps]`：聚合 scorecard、benchmark status 和最近 goal readiness，输出本轮产品迭代状态、去重后的门禁和下一步动作。
- `deepcli benchmark presets|run-suite|run|record|status|gate|summary|trends|baseline-template|compare|list|show|clean [--json]`：发现推荐 workload、一键执行推荐基准套件、执行单项 preset、记录、评估证据质量、门禁、汇总、趋势分析、baseline 模板、baseline 对比、列出、查看和清理本地 benchmark 证据 artifact。

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
- `/goal` 可把当前会话绑定到长期目标，默认目标是完整实现项目文档需求，并要求验收命令和测试全部通过后才可结束。
- `/fork` 会复制当前或指定会话目录中的持久化上下文，给副本生成新 id/title，并默认打开新 macOS Terminal 执行 `deepcli resume <new_id>`；当前运行中的 Agent 任务分叉暂不宣称支持，建议等待或先 `/stop`。
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

- `deepcli recipes sota --json`
- `deepcli recipes release --json`
- `deepcli goal "完整实现当前项目文档中的全部需求" --json`
- `deepcli goal status --json`
- `deepcli goal gate --json`
- `deepcli plan "做一个需求澄清功能" --write-doc docs/ai/PLANNED_REQUIREMENTS.md`
- `deepcli fork --current --no-open --json`
- `deepcli scorecard --json`
- `deepcli round --json`
- `deepcli round --json --run-benchmark --fail-on-command`
- `deepcli benchmark presets --json`
- `deepcli benchmark status --json`
- `deepcli benchmark gate --json`
- `deepcli benchmark run-suite --json --fail-on-command`
- `deepcli benchmark summary --json`
- `deepcli benchmark trends --json`
- `deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`
- `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`
- `deepcli benchmark clean --dry-run --json`
- `deepcli test discover --json`
- `deepcli test run --json -- cargo test`
- `deepcli accept --json`
- `deepcli gate --json`
- `deepcli verify --json`
- `deepcli handoff --pr`
- `deepcli preflight --json`

验收报告会聚合 Git 状态、diff、review 风险、测试证据、环境证据、失败工具、待审批和会话信号。无当前会话的一次性 `accept` / `gate` 会优先使用本次 workspace 测试证据，避免历史 session 的旧失败污染最终验收。

`goal` 输出稳定 `deepcli.goal.v1` JSON，并把目标、需求来源、停止条件和验收命令保存到当前 session 的 `goal.json` 与守护 `plan.json`。后续 Provider 上下文会收到 active goal contract，约束 Agent 不能在目标、验收要求和测试全部满足前声称结束。`goal status` 输出稳定 `deepcli.goal.status.v1`，检查需求来源文件、goal 守护计划步骤和每条 acceptance command 的最新测试证据；`goal gate` 复用同一报告，并在仍有 blocker 时返回非零，适合用作“是否允许停止”的本地门禁。`goal show/status/gate` 在无 active session 或当前 session 没有 goal 时，会回退到最近一个带 goal 的会话，并在 JSON 中标注 `sessionSource`；创建和清理 goal 不回退，避免 one-shot 命令误写历史会话。`plan` 输出稳定 `deepcli.plan.requirements_draft.v1`，面向粗糙需求生成澄清问题、多个候选选项、首推选项、假设、功能要求、验收标准和下一步动作；在有当前 session 时，澄清问题也会进入旁路问题队列，用户可继续回答。`fork` 输出稳定 `deepcli.session.fork.v1`，复制已持久化会话上下文但不复制 metadata id，适合把同一上下文分支给新的终端继续探索；当前运行中的后台 Agent 分叉先作为明确限制处理。

`preflight` / `release-check` 是提交/推送前的一键本地检查入口，串联 `cargo fmt --check`、`git diff --check`、`cargo clippy --all-targets -- -D warnings`、`selftest`、`doctor --quick`、`privacy --fail-on-findings` 和 `gate --json`，并输出稳定 JSON 报告；`--dry-run` 只预览检查清单，`--quick` 跳过较慢的 clippy/gate。

`recipes` / `playbook` 是任务型工作流目录，按 start、code、debug、release、support、environment、shell、sota 等主题输出可复制命令和稳定 `deepcli.recipes.v1` JSON，适合 TUI、外部 UI 或团队脚本引导用户选择下一步；`recipes.nextActions` 保持为可直接执行的 `deepcli ...` 命令，说明性上下文留在 `recipes[].notes` 和 report 中；`recipes sota` 可通过 `product-loop`、`benchmark` 或 `round` alias 进入，用于把产品缺口检查、benchmark evidence、baseline 模板、baseline compare 和 gate 串成一条本地闭环；该命令本地只读，不创建 session、不调用 Provider。

`scorecard` 是产品能力评分和 SOTA 差距入口，按命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据给出 0-100 分、tier、gaps、next actions 和稳定 `deepcli.scorecard.v1` JSON；`--fail-below` 可作为本地产品门禁，命令不创建 session、不调用 Provider。`scorecard.nextActions` 在存在 gaps 时会作为本轮修复队列，只聚焦当前 gaps 的直接修复、SOTA 产品循环和验收动作；各分类的 `nextActions` 仍会先展示本分类 gap 的修复动作，再展示通用探索命令；当只缺 benchmark evidence 时，首项直接指向可复制执行的 `deepcli round --json --run-benchmark --fail-on-command` 修复命令，不再把当前 `deepcli scorecard --json` 报告本身塞回 benchmark evidence 修复队列，并继续露出 `deepcli recipes sota --json` 作为完整产品循环导航。`round` 默认输出稳定 `deepcli.round.v1`，把 scorecard、benchmark status 和最近 goal readiness 聚合成本轮产品迭代报告，包含 ready 状态、门禁、gaps 和下一步命令；内嵌的 `scorecard.categories[]` 摘要会保留分类级 `nextActions`，让 TUI、外部 UI 或脚本只读取 round 报告也能按分类展示修复动作；`scorecard` gate 只检查分数阈值是否达标，benchmark evidence 和 goal readiness 由专属 gate 呈现，其它未满足项继续留在 gaps 列表中，避免同一问题在多个 gate 中重复失败；benchmark gate 会列出缺失、weak、stale、失败或超时的 required preset，让用户在同一份 round 报告里直接看到证据缺口。`round.nextActions` 按失败 gate 的修复路径排序；当 scorecard 已达标且剩余 gaps 全部属于 benchmark evidence 时，首个动作是 `deepcli round --json --run-benchmark --fail-on-command`，并省略重复的 `deepcli scorecard --json`，同时给出 `deepcli recipes sota --json`。存在未 ready 的 goal 时会输出 `goalStatus` 摘要、`goal_readiness` gate 和 `deepcli goal gate --json` 下一步动作，没有 goal 时保持只读报告且不创建 session。显式加 `--run-benchmark` 或 `--run-suite` 时会先执行 benchmark suite，再在同一份 round JSON 中写入 `benchmarkRun` 和更新后的 `benchmarkStatus`；`--fail-on-command` 适合阻断 benchmark 命令失败，`--fail-on-gaps` 适合在持续产品循环或 CI 中要求本轮 evidence、产品分数和 goal readiness 都 ready。`benchmark` 保留无子命令和 scorecard flags 的兼容行为，并增加 `presets/run-suite/run/record/status/gate/summary/trends/baseline-template/compare/list/show/clean`：`presets` 列出 cargo-test、preflight-quick、selftest、scorecard 和 smoke 等推荐 workload，`run-suite` 默认连续执行 cargo-test、preflight-quick、selftest 和 scorecard，并输出稳定 `deepcli.benchmark.suite.v1` 汇总报告，也可重复传入 `--preset` 只跑指定子集，`run --preset <name>` 显式执行对应本地命令、采集 exit code、耗时和输出摘要并写入 `.deepcli/benchmarks/*.json`，`record` 只记录声明证据，`status` 输出稳定 `deepcli.benchmark.status.v1` 并把证据判定为 missing、weak、incomplete、failing、stale 或 ready，同时展示 required preset 覆盖细节，gap 修复提示使用可直接执行的 `deepcli benchmark ...` 命令，避免单项通过被误判为完整 suite 证据，证据缺失时优先在 nextActions 中给出 `deepcli recipes sota --json`，`gate` 在 status 不是 ready 时返回非零，`summary` 聚合历史 artifact 的通过率、失败数、耗时范围和最新 artifact，`trends` 输出稳定 `deepcli.benchmark.trends.v1`，按 suite/case 展示最近状态回归、恢复和耗时变化，`baseline-template` 输出可编辑的 `deepcli.benchmark.baseline.v1` JSON 并可写入 workspace 内 baseline 文件，`compare` 输出稳定 `deepcli.benchmark.compare.v1`，只读取本地 artifact 和 workspace 内 baseline JSON，按 suite/case 展示状态对比、缺失项和耗时差异；baseline 仍缺 `status` 或 `durationMs` 时，`compare` 会保持 `incomplete` 并在 nextActions 中提示先编辑对应 baseline 文件，`list/show` 用于本地验收和持续产品循环，`clean` 输出稳定 `deepcli.benchmark.cleanup.v1`，默认 dry-run 预览旧 artifact，只有显式 `--force` 才删除。

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
./scripts/deepcli help goal
./scripts/deepcli help plan
./scripts/deepcli help fork
./scripts/deepcli recipes release --json
./scripts/deepcli scorecard --json
./scripts/deepcli round --json
./scripts/deepcli round --json --run-benchmark --fail-on-command
./scripts/deepcli benchmark presets --json
./scripts/deepcli benchmark list --json
./scripts/deepcli benchmark status --json
./scripts/deepcli benchmark gate --json
./scripts/deepcli benchmark run-suite --json --fail-on-command
./scripts/deepcli benchmark summary --json
./scripts/deepcli benchmark trends --json
./scripts/deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark clean --dry-run --json
./scripts/deepcli preflight --json
./scripts/deepcli release-check --dry-run
```

## 后续方向

持续改进方向包括：

- 更强的 TUI 信息架构和任务观察面板。
- 更完整的自动环境准备与 smoke test。
- 更智能的 session 恢复、搜索和交接。
- 更系统的 provider 延迟、上下文压缩和工具失败诊断。
- 更正式的端到端任务集和横向模型/工具对比。
- 更接近 SOTA 编程代理的端到端任务闭环。
