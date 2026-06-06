# deepcli

deepcli 是一个 local-first 的 AI 编程代理 CLI，面向日常工程协作场景：启动 TUI、切换 Provider/模型、恢复会话、检查健康状态、准备本地环境、运行测试，以及生成验收或交付报告。

本文是快速入口。完整功能介绍持续更新在 [docs/FEATURES.md](docs/FEATURES.md)。

## 当前状态

产品仍在快速迭代中，命令面和交互体验会持续扩展。文档中的功能清单以当前已经落地并可验收的能力为准。

## 快速开始

构建二进制：

```bash
cargo build
```

在当前项目中启动 deepcli：

```bash
./scripts/deepcli
```

如果 `deepcli` 已经在 `PATH` 中：

```bash
deepcli
```

运行本地自检，不调用 Provider：

```bash
deepcli selftest --json
deepcli doctor --quick --json
deepcli doctor shell --json
deepcli health --json
```

配置凭据：

```bash
printf '%s' "$DEEPSEEK_API_KEY" | deepcli login deepseek --stdin --force
deepcli credentials status --json
```

切换 Provider 或模型：

```bash
deepcli use deepseek deepseek-v4-pro
deepcli use kimi kimi-for-coding
deepcli model list --json
```

恢复历史任务：

```bash
deepcli resume
deepcli sessions --all --limit 20
```

设置长期目标、澄清需求或复制会话：

```bash
deepcli goal "完整实现当前项目文档中的全部需求" --json
deepcli goal status --json
deepcli goal gate --json
deepcli plan "做一个可以交互式澄清需求的功能" --write-doc docs/ai/PLANNED_REQUIREMENTS.md
deepcli fork --current --no-open --json
```

## 常用工作流

启动交互式编程会话：

```bash
deepcli
deepcli deepseek
deepcli kimi
```

执行一次性请求：

```bash
deepcli ask "阅读项目结构并说明如何运行测试"
deepcli stream "请只回答 OK"
```

检查并验收工作区：

```bash
deepcli status --json
deepcli doctor --quick --json
deepcli usage --json
deepcli trace --limit 30
deepcli logs --limit 80
deepcli privacy --json
deepcli recipes release --json
deepcli scorecard --json
deepcli round --json
deepcli round --json --run-benchmark --fail-on-command
deepcli preflight --json
deepcli accept --json
deepcli gate --json
deepcli handoff --pr
```

`goal` 会在当前会话中写入目标契约和守护计划，后续 Agent 上下文会持续看到验收条件，只有目标达成、要求验收通过且测试通过后才可结束；`goal status` 会检查文档来源、计划步骤和 acceptance command 的测试证据，`goal gate` 在仍有 blocker 时返回非零。无 active session 时，`goal show/status/gate` 会回退到最近一个带 goal 的会话；创建或清理 goal 仍要求 active session，避免误写历史会话。`plan` 面向不成熟需求，生成带推荐选项的澄清问题、假设、功能要求和验收标准，并可写成需求草稿。`fork` 会复制已持久化的会话上下文，默认在新 macOS Terminal 中执行 `deepcli resume <new_id>`；`--no-open` 可用于脚本或验收。

无当前会话时，`accept` / `gate` 会使用本次 workspace 测试证据，不会被历史 session 的旧失败记录污染。

查看任务型工作流清单：

```bash
deepcli recipes
deepcli recipes sota --json
deepcli recipes release --json
deepcli playbook support
deepcli scorecard --json
deepcli round --json
deepcli round --json --run-benchmark --fail-on-command
deepcli round --json --fail-on-gaps
deepcli benchmark --fail-below 85
deepcli benchmark presets --json
deepcli benchmark status --json
deepcli benchmark gate --json
deepcli benchmark run-suite --json --fail-on-command
deepcli benchmark run --preset cargo-test --json --fail-on-command
deepcli benchmark record --json --suite product --case scorecard
deepcli benchmark list --json
deepcli benchmark summary --json
deepcli benchmark trends --json
deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json
deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
deepcli benchmark clean --dry-run --json
```

`recipes` / `playbook` 是本地只读入口，用于按 start、code、debug、release、support、environment、shell、sota 等主题查看可复制命令，不创建 session、不调用 Provider；`recipes sota` 也可通过 `product-loop`、`benchmark` 或 `round` alias 进入，用于串起 scorecard、round、benchmark evidence、baseline 模板、baseline compare 和 benchmark gate。

`scorecard` 是本地只读产品能力评分入口，用于按命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据查看 SOTA 差距；支持稳定 `deepcli.scorecard.v1` JSON、workspace 内 `--output` 和 `--fail-below` 门禁。`scorecard.nextActions` 会把当前 gaps 的修复动作排在通用探索命令之前；各分类的 `nextActions` 也会先展示本分类 gap 的修复动作，再展示通用探索动作；当唯一缺口是 benchmark evidence 时，首个动作会指向 `/round --json --run-benchmark --fail-on-command` 修复动作，并继续露出 `deepcli recipes sota --json` 作为完整产品循环导航。`round` 默认聚合 scorecard、benchmark status，并在存在 goal 时纳入最近 goal readiness，输出稳定 `deepcli.round.v1`，用于每轮产品设计/工程实现后的迭代复盘和下一步动作判断；`round` 内嵌的 `scorecard.categories[]` 摘要会保留分类级 `nextActions`，让 TUI 或外部 UI 只读取一份 round 报告也能按分类展示修复动作；`scorecard` gate 只表示分数是否达到本轮阈值，benchmark evidence、goal readiness 和其它 gaps 会分别在专属 gate 或 gaps 列表中呈现，避免同一缺口重复标红；benchmark gate 会直接列出缺失、weak、stale、失败或超时的 required preset，用户无需再打开第二份报告才能知道该补哪些证据。`round.nextActions` 会优先给出失败 gate 的直接修复命令；当 scorecard 分数已过线且只剩 benchmark evidence 缺口时，下一步会直接指向 `deepcli round --json --run-benchmark --fail-on-command`，不再把 `deepcli scorecard --json` 放在前面，并把 `deepcli recipes sota --json` 放进同一组后续动作。有未 ready 的 goal 时，JSON 会包含 `goalStatus` 摘要和 `goal_readiness` gate，并提示 `deepcli goal gate --json`。显式加 `--run-benchmark` 或 `--run-suite` 时会先执行 benchmark suite，再在同一份 round JSON 中写入 `benchmarkRun` 和更新后的 `benchmarkStatus`；`--fail-on-command` 可在 benchmark 命令失败时返回非零，`--fail-on-gaps` 可让 CI 在本轮证据、分数或 goal readiness 未 ready 时失败。`benchmark` 保留 scorecard 兼容参数，同时支持 `presets/run-suite/run/record/status/gate/summary/trends/baseline-template/compare/list/show/clean` 在 `.deepcli/benchmarks/` 下发现推荐 workload、一键执行推荐基准套件、执行单项 preset、记录、评估证据质量、门禁、汇总、趋势分析、baseline 模板、baseline 对比、查看和清理稳定 `deepcli.benchmark.record.v1` / `deepcli.benchmark.suite.v1` / `deepcli.benchmark.status.v1` / `deepcli.benchmark.summary.v1` / `deepcli.benchmark.trends.v1` / `deepcli.benchmark.compare.v1` / `deepcli.benchmark.cleanup.v1` 证据 artifact；`run-suite` 默认执行 cargo-test、preflight-quick、selftest 和 scorecard，也可重复传入 `--preset` 指定子集；`status` 会把证据分为 missing、weak、incomplete、failing、stale 或 ready，并在 JSON 中展示 required preset 覆盖细节，避免只跑单个 cargo-test 或 smoke artifact 就被当作完整 benchmark；证据缺失时，`benchmark status.nextActions` 会优先给出 `deepcli recipes sota --json`，帮助用户回到完整产品循环；`trends` 可按 suite/case 展示最近状态回归和耗时变化；`baseline-template` 生成可直接编辑的 `deepcli.benchmark.baseline.v1` JSON，`--output` 会写入 workspace 内 baseline 文件；`compare` 只读取本地 artifact 和 workspace 内 baseline JSON，按 suite/case 输出状态回归、恢复、缺失和耗时差异，不执行 shell、不调用 Provider；当 baseline 仍缺 `status` 或 `durationMs` 时，`compare` 会保持 `incomplete` 并在 nextActions 中提示先编辑对应 baseline 文件；`gate` 等价于 `status --fail-on-not-ready`，便于 CI 或发布脚本在证据不足时返回非零；`clean` 默认 dry-run，可用 `--force --keep n` 或 `--older-than-days n` 删除旧本地 artifact；该目录默认本地忽略，不会误提交凭据或机器路径。

准备本地环境：

```bash
deepcli check docker --json
deepcli env plan compiler --smoke --json
deepcli setup docker --smoke
deepcli env test compiler --json
```

安装或检查 shell completion：

```bash
deepcli completion status zsh --json
deepcli completion install zsh --force
deepcli completion json --output .deepcli/exports/commands.json
```

## 文档

- [功能介绍](docs/FEATURES.md)：面向用户的能力清单，持续更新中。
- [需求文档](docs/ai/REQUIREMENTS.md)：产品需求和目标行为。
- [技术计划](docs/ai/TECHNICAL_PLAN.md)：架构与实现说明。

## 本地验证

常用检查命令：

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
./scripts/deepcli selftest --json
./scripts/deepcli doctor --quick --json
./scripts/deepcli help goal
./scripts/deepcli help plan
./scripts/deepcli help fork
./scripts/deepcli scorecard --json
./scripts/deepcli round --json
./scripts/deepcli round --json --run-benchmark --fail-on-command
./scripts/deepcli benchmark list --json
./scripts/deepcli benchmark status --json
./scripts/deepcli benchmark gate --json
./scripts/deepcli benchmark run-suite --json --fail-on-command
./scripts/deepcli benchmark summary --json
./scripts/deepcli benchmark trends --json
./scripts/deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark clean --dry-run --json
./scripts/deepcli preflight --dry-run
./scripts/deepcli release-check --dry-run
./scripts/deepcli preflight --json
```

`selftest` 和 `doctor` 会读取 `.deepcli/config.json` 中的 `project.gitIdentity`，对比当前 Git 仓库的有效 `user.name` / `user.email`，用于提交前发现错误作者身份。

`preflight` / `release-check` 是提交或推送前的一键本地检查入口，会串联格式、diff whitespace、clippy、selftest、doctor、privacy 和 gate；`--dry-run` 可先预览将执行的检查，`--quick` 可跳过较慢的 clippy/gate。

`privacy.allowedEmails` / `privacy.allowedEmailDomains` 可声明公开或允许的邮箱，让 `deepcli privacy` 将这些命中记录为 suppressed findings，而不是阻断开源前检查；只想允许提交元数据时可使用 `privacy.allowedCommitEmails` / `privacy.allowedCommitDomains`。
`privacy.allowedUserPaths` 可声明脱敏后的历史本机用户路径，用于折叠已知迁移遗留路径。

## 仓库

当前 GitHub 远程仓库：

```text
https://github.com/zero-kotori/deepcli
```
