# deepcli 技术方案

## 1. 设计目标

`deepcli` 采用本地优先、权限受控、可恢复的 Agent Runtime 设计。系统需要支持 DeepSeek API，并通过统一 provider 接口扩展到 Kimi、OpenAI、Anthropic 或本地模型。CLI 要能执行完整编程代理链路：理解项目、制定计划、调用工具、修改文件、运行测试、修复问题、review、保存会话、可选 Git 提交。

## 2. 总体架构

建议采用分层模块：

- `cli`：命令入口、参数解析、一次性任务、交互式入口。
- `ui`：REPL/message box/TUI 渲染、快捷键、slash 命令提示、任务观察面板、审批交互、状态展示。
- `runtime`：Agent 主循环、状态机、任务计划、工具调度、子 Agent 调度。
- `providers`：DeepSeek、Kimi、其他 provider 适配器。
- `tools`：文件、shell、git、网络、测试、skill、终端等工具。
- `environment`/`tools`：本地环境检测、Docker/Colima 安装配置、任务镜像拉取和 smoke test，所有动作仍通过权限引擎审计。
- `permissions`：权限策略、sandbox、风险分级、审批流、auto-reviewer。
- `workspace`：项目扫描、ignore、隐私过滤、上下文打包。
- `session`：会话保存、恢复、trace、历史记录。
- `skills`：Skill 生成、注册、发现、调用。
- `prompts`：内置 prompt、自定义 prompt、system prompt 模板。
- `config`：项目配置、用户配置、provider 凭据、代理配置。

关键原则：Agent 不能直接访问文件系统、shell、网络和 Git，所有动作必须通过工具系统和权限引擎。

CLI 入口在构造 `AgentRuntime` 前应识别本地 one-shot slash 命令。`/help` 和 `/quickstart` 可在 workspace 授权前直接返回；`/recipes`、`/scorecard`、`/round`、`/benchmark`、`/selftest`、`/preflight`、`/completion`、`/init`、`/version`、`/about`、`/diagnose`、`/doctor`、`/health`、`/status`、`/usage`、`/next`、`/accept`、`/gate`、`/verify`、`/handoff`、`/trace`、`/logs`、`/context`、`/permissions show`、`/credentials status|template|import-env|set|remove`、`/login`、`/logout`、`/auth`、`/apikey`、`/key`、`/config show|sources|validate|get`、`/timeout show|set|reset`、`/model show|list|set`、`/model <provider>`、`/provider`、`/use`、`/switch`、`/models`、`/providers`、`/docker`、`/compiler`、`/prompt list|get|render`、`/skill list|run`、`/agent list|show`、`/history`、`/cleanup`、`/session list|search|next|diagnose|show|history|summary|tools|tests|diffs|backups|export` 和无 id 的 `/resume` 通过无 session 的 `CommandContext` 执行，避免为了初始化、自检、产品评分、产品迭代轮次、benchmark artifact、发布前检查、补全、查看帮助、版本/支持元数据、状态、用量、trace、配置、凭据状态/写入/移除、provider 超时查看/调整、模型列表/切换、环境检查、prompt/skill/agent 清单或历史检索而产生空会话；凭据写入/移除命令、模型配置切换命令和 provider turn 超时调整命令可以写 `.deepcli/credentials` 或 `.deepcli/config.json`，但不能创建 session 或先调用 provider；只读 one-shot 命令使用 `--yes` 时应使用临时 workspace 授权，不能仅因授权写入 `.deepcli/authorization.json`。`/completion` 不带 `--output` 时可作为 authorization-free 静态输出；带 `--output` 时通过 workspace path 校验写入当前格式。`/init` 和 `/doctor --fix` 允许创建 `.deepcli/` 本地结构，但不能创建 session 记录；`/doctor --quick` 或 `--no-env` 跳过可能较慢的 Docker/Colima 环境检查。无一次性任务时，Rust 二进制和启动脚本都应默认进入 TUI；旧行式 REPL 通过 `--repl` 或 `deepcli repl` 显式进入。Rust CLI 本体和启动 wrapper 都负责把高频顶层子命令映射到 slash 命令，例如 `deepcli quickstart` -> `/quickstart`、`deepcli recipes release --json` -> `/recipes release --json`、`deepcli playbook support` -> `/playbook support`、`deepcli scorecard --json` -> `/scorecard --json`、`deepcli round --json` -> `/round --json`、`deepcli benchmark --fail-below 85` -> `/benchmark --fail-below 85`、`deepcli benchmark presets --json` -> `/benchmark presets --json`、`deepcli benchmark run-suite --json` -> `/benchmark run-suite --json`、`deepcli benchmark run --preset cargo-test --json` -> `/benchmark run --preset cargo-test --json`、`deepcli benchmark record --json` -> `/benchmark record --json`、`deepcli benchmark list --json` -> `/benchmark list --json`、`deepcli benchmark summary --json` -> `/benchmark summary --json`、`deepcli benchmark trends --json` -> `/benchmark trends --json`、`deepcli benchmark clean --dry-run --json` -> `/benchmark clean --dry-run --json`、`deepcli selftest --json` -> `/selftest --json`、`deepcli preflight --json` -> `/preflight --json`、`deepcli release-check --dry-run` -> `/release-check --dry-run`、`deepcli completion zsh` -> `/completion zsh`、`deepcli completion json` -> `/completion json`、`deepcli version --json` -> `/version --json`、`deepcli about --json` -> `/about --json`、`deepcli health --json` -> `/health --json`、`deepcli doctor --quick` -> `/doctor --quick`、`deepcli doctor docker --json` -> `/env check docker --json`、`deepcli next` -> `/next`、`deepcli diagnose` -> `/diagnose`、`deepcli diagnose compiler --json` -> `/env check compiler --json`、`deepcli models --json` -> `/model list --json`、`deepcli providers --json` -> `/model list --json`、`deepcli use kimi` -> `/model set kimi`、`deepcli switch deepseek deepseek-v4-pro` -> `/model set deepseek deepseek-v4-pro`、`deepcli provider kimi` -> `/model set kimi`、`deepcli provider --json` -> `/model show --json`、`deepcli history --limit 10` -> `/session list --limit 10`、`deepcli cleanup sessions --json` -> `/cleanup sessions --json`、`deepcli accept --json` -> `/accept --json`、`deepcli gate --json` -> `/gate --json`、`deepcli login deepseek --stdin` -> `/credentials set deepseek --stdin`、`deepcli logout deepseek` -> `/credentials remove deepseek`、`deepcli timeout 900` -> `/timeout 900`、`deepcli timeout --json` -> `/timeout --json`、`deepcli auth --stdin` -> `/credentials set <default-provider> --stdin`、`deepcli check docker --json` -> `/check docker --json`、`deepcli docker --json` -> `/env check docker --json`、`deepcli compiler setup --smoke` -> `/env setup compiler --smoke`、`deepcli test docker --json` -> `/env test docker --json`、`deepcli test run` -> `/test run`、`deepcli setup docker --smoke` -> `/setup docker --smoke`、`deepcli install compiler --smoke` -> `/install compiler --smoke`、`deepcli verify` -> `/verify`、`deepcli handoff` -> `/handoff`、`deepcli help doctor` -> `/help doctor`、`deepcli session history --limit 20` -> `/session history --limit 20`、`deepcli sessions --all` -> `/session list --all`；也要在构造 `AgentRuntime` 前识别 provider 和模式别名，例如 `deepcli deepseek quickstart`、`deepcli deepseek recipes release --json`、`deepcli deepseek scorecard --json`、`deepcli deepseek round --json`、`deepcli deepseek benchmark presets --json`、`deepcli deepseek benchmark run-suite --json`、`deepcli deepseek benchmark list --json`、`deepcli deepseek benchmark summary --json`、`deepcli deepseek benchmark trends --json`、`deepcli deepseek benchmark clean --dry-run --json`、`deepcli deepseek selftest --json`、`deepcli deepseek preflight --json`、`deepcli deepseek release-check --dry-run`、`deepcli deepseek completion json`、`deepcli deepseek version --json`、`deepcli deepseek about --json`、`deepcli deepseek health`、`deepcli deepseek providers`、`deepcli deepseek use`、`deepcli deepseek switch kimi`、`deepcli deepseek provider --json`、`deepcli deepseek history`、`deepcli deepseek cleanup sessions --json`、`deepcli deepseek accept --json`、`deepcli deepseek gate --json`、`deepcli deepseek login`、`deepcli deepseek logout`、`deepcli deepseek auth --stdin`、`deepcli deepseek timeout 900`、`deepcli deepseek doctor --quick`、`deepcli deepseek doctor docker`、`deepcli deepseek diagnose`、`deepcli deepseek diagnose compiler`、`deepcli deepseek check docker`、`deepcli deepseek docker`、`deepcli deepseek compiler setup --smoke`、`deepcli deepseek test compiler`、`deepcli deepseek setup docker --smoke`、`deepcli deepseek help doctor`、`deepcli deepseek logs --limit 80`、`deepcli deepseek stream <prompt>`、`deepcli resume [session_id]`，避免二进制直连时把这些入口误当普通 prompt 发给模型。启动 wrapper 不能剥掉 `ask`/`stream` 模式词，应交给 Rust 本体统一校验和归一化，避免破坏 `deepcli ask ...` 的逃生路径。`ask` 和 `stream` 模式缺少 prompt 时必须本地报错，不能退回 TUI、创建 session 或调用 provider。对于 `deepcli doctro --quick`、`deepcli deepseek doctro --quick` 这类明显像拼错的顶层命令，应在本地返回 nearest-command 建议和 `deepcli ask ...` 逃生提示，不创建 session，不调用 provider。

顶层别名后接 `--help` 或 `-h` 时，Rust CLI 本体和启动 wrapper 都应在构造 runtime 前转发到对应帮助主题，例如 `deepcli fork --help` -> `/help fork`、`deepcli sessions -h` -> `/help session`、`deepcli providers --help` -> `/help model`、`deepcli deepseek fork --help` -> `/help fork`，并保留 provider 前缀的 provider/model override。

`deepcli benchmark run-suite --json`、`deepcli benchmark status --json`、`deepcli benchmark gate --json`、`deepcli benchmark trends --json`、`deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`、`deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`、`deepcli benchmark clean --dry-run --json`、provider 前缀下的 `deepcli deepseek benchmark run-suite --json` / `deepcli deepseek benchmark status --json` / `deepcli deepseek benchmark gate --json` / `deepcli deepseek benchmark trends --json` / `deepcli deepseek benchmark baseline-template --output .deepcli/baselines/competitor.json --json` / `deepcli deepseek benchmark compare --baseline .deepcli/baselines/competitor.json --json` / `deepcli deepseek benchmark clean --dry-run --json` 和 slash 入口 `/benchmark run-suite --json` / `/benchmark status --json` / `/benchmark gate --json` / `/benchmark trends --json` / `/benchmark baseline-template --output .deepcli/baselines/competitor.json --json` / `/benchmark compare --baseline .deepcli/baselines/competitor.json --json` / `/benchmark clean --dry-run --json` 也必须在构造 runtime 前映射为本地命令，复用 workspace 输出路径校验，不创建 session、不调用 provider；`run-suite` 只执行显式 benchmark preset 集合并写入本地 artifact，`status/gate/trends/baseline-template/compare/clean --dry-run` 不执行 shell，`gate` 等价于 `status --fail-on-not-ready`，用于 CI 或发布脚本在 benchmark 证据缺失、弱、失败或过期时返回非零。

`/privacy` 和 `deepcli privacy` 也属于无 session、本地只读入口；provider 前缀下的 `deepcli deepseek privacy --json` 应映射到同一命令。该入口用于开源或共享前的 Git history 隐私审计，不创建 session、不调用 provider。

`/recipes`、`/recipe`、`/playbook`、`/workflow` 和 `deepcli recipes|playbook|workflow ...` 也属于无 session、本地只读入口；provider 前缀下的 `deepcli deepseek recipes release --json` 和 `deepcli deepseek recipes sota --json` 应在构造 runtime 前映射到同一命令。该入口用于把 start、code、debug、release、support、environment、shell、sota 等任务型工作流暴露给 TUI、外部 UI、团队脚本和人工用户，避免用户在完整 help 中查找高频命令；`nextActions` 输出可直接执行的 `deepcli ...` 命令，说明性上下文留在 notes 和 report 中；`sota` 主题也接受 product-loop、benchmark、round 等 alias，用于串联 scorecard、round、benchmark evidence、baseline 模板、baseline compare 和 benchmark gate；该主题的顶层 `nextActions` 应复用当前 `round` 的状态感知修复队列，并补充 baseline compare，避免产品循环入口先推荐已知无法完成闭环的只读报告。

`/scorecard`、`/sota` 和 `deepcli scorecard ...` 也属于无 session、本地只读入口；provider 前缀下的 `deepcli deepseek scorecard --json` 应在构造 runtime 前映射到同一命令。该入口用于把“接近 SOTA”拆成命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据等可执行评分项，避免产品循环只靠人工主观判断。`build_scorecard_report` 应把 `scorecard_add_gap` 产生的 remediation actions 单独保存在 category priority actions 中，每个 category 的 `nextActions` 先合并这些 gap 修复动作再追加本分类普通 discovery actions；构造全局 `nextActions` 时，有 gaps 则先展开这些 gap 修复动作，再追加有 gap category 的动作和固定验收动作；没有 gaps 且状态为 ok 时，全局 `nextActions` 应切换为持续验收动作，不应重复混入各强分类普通 discovery actions；`deepcli.scorecard.v1` 顶层 `checklist[]` 从全局 `nextActions` 中可直接执行的命令派生 step/label/command，用于 UI 渲染本轮全局动作队列，分类级 `categories[].checklist[]` 继续表示分类动作；若 benchmark evidence 已 ready 但 `benchmark trends` 仍是 `insufficient_history` 或 `regression`，全局首项应切换为 `deepcli round --json --run-benchmark --fail-on-command`，随后保留 SOTA recipe、trends/status、preflight/gate 和 baseline compare；因此只有 benchmark evidence gap 时，`scorecard.nextActions[0]` 和 `benchmark_evidence.nextActions[0]` 都应指向 `deepcli round --json --run-benchmark --fail-on-command` 这类可直接执行的 CLI 修复命令，而不是 `deepcli quickstart --json`、`deepcli scorecard --json` 或 `deepcli round --json` 之类通用入口；同一份 scorecard 报告的全局动作和 `benchmark_evidence.nextActions` 都不应包含 `deepcli scorecard --json` 自引用动作或 `deepcli round --json` 只读报告动作，并继续包含 `deepcli recipes sota --json`，让用户能从评分报告进入完整产品循环导航。`/benchmark`、`/bench` 和 `deepcli benchmark ...` 属于同一类本地入口：无子命令或 scorecard flags 时兼容 `/scorecard`，`presets/run-suite/run/record/status/gate/summary/trends/list/show/clean` 用于发现推荐 workload、一键执行推荐基准套件、执行单项 preset、沉淀、评估证据质量、门禁、汇总、趋势分析、查看和清理 `.deepcli/benchmarks/*.json` 证据 artifact；`presets` 只读展示可执行 preset，`run-suite` 默认执行 meaningful preset 集合，输出稳定 `deepcli.benchmark.suite.v1` 并可用重复 `--preset` 或 `--presets a,b` 限定子集，`run` 只能在用户显式提供 preset 或命令时执行本地 shell 并受超时约束，`record` 只能记录声明的命令和本地摘要，`status/gate/summary/trends/list/show/clean` 只读取本地 artifact，不能调用 provider；`trends` 输出最近 per-case 状态回归、恢复和耗时变化，若已有 artifact 但没有任何 case 具备 previous 样本则返回 `insufficient_history` 并优先提示 `deepcli round --json --run-benchmark --fail-on-command`，同时保留 benchmark run-suite 作为底层补样本动作；`clean` 默认 dry-run，只有显式 `--force` 才删除旧 artifact；`status` 输出稳定 `deepcli.benchmark.status.v1`，区分 missing、weak、incomplete、failing、stale 和 ready，并用 `presetCoverage.requiredStatus` 强制展示必需 preset 覆盖，gap 修复提示必须使用可直接执行的 `deepcli benchmark ...` 命令，避免 smoke-only 或单个 meaningful artifact 被误判为完整 benchmark 证据；`status.nextActions` 应包含 `deepcli recipes sota --json`，并在 artifact_count 为 0 时隐藏 `deepcli benchmark clean --dry-run --json`，只有已有本地 artifact 时才展示该 dry-run 清理动作；`gate` 在 status 不是 ready 时通过 `CommandExit` 返回非零但保留报告。

`/round`、`/iterate` 和 `deepcli round ...` 属于无 session、本地产品循环入口；provider 前缀下的 `deepcli deepseek round --json` 和 `deepcli deepseek round --json --run-benchmark --fail-on-command` 应在构造 runtime 前映射到同一命令。该入口用于把每轮“产品设计师检查 -> 工程实现 -> 验证”的状态产品化：默认复用 `build_scorecard_report`、`build_benchmark_status_report`，并在存在 goal 时通过 `select_goal_session(None)` + `collect_goal_readiness` 读取最近 goal readiness，输出稳定 `deepcli.round.v1`，包含 scorecard 摘要、benchmarkStatus 摘要、可选 `goalStatus`、gates、gaps、nextActions 和 report；`scorecard` gate 使用 `scorecard.percent >= score_threshold` 判断分数阈值是否达标，scorecard gaps 仍进入总 gaps，benchmark evidence 和 goal readiness 由专属 gate 判断，避免一个 benchmark 缺口同时让 scorecard gate 与 benchmark gate 都失败；benchmark evidence gate 从 `required_preset_statuses` 汇总 missing、weak、stale、failed 和 timeout preset 名称并追加到 summary；`nextActions` 按失败 gate 的直接修复动作排序，只有 `!scorecard_threshold_ready` 或 `scorecard_has_standalone_round_gaps(scorecard)` 为真时才加入 `deepcli scorecard --json`，因此 scorecard 已过线且仅剩 `benchmark_evidence:` gaps 时首个动作是 `deepcli round --json --run-benchmark --fail-on-command`，后续包含 `deepcli recipes sota --json`，并且外层 `nextActions` 不包含刚运行的 `deepcli round --json` 自引用动作；当 `status == ready` 时，外层 `nextActions` 在 preflight/gate 后追加 `sota_baseline_next_actions(workspace)`，默认 baseline 存在时提示 baseline compare，缺默认 baseline 且当前 artifact 可完整捕获时先提示 `baseline-template --from-current`，再提示生成 `.deepcli/baselines/competitor.json`；没有 goal 时不创建 session、不生成 goal gate，存在未 ready goal 时追加 `goal_readiness` gate 和 `deepcli goal gate --json` 下一步动作。显式传入 `--run-benchmark`/`--run-suite`、`--preset`、`--presets`、`--fail-on-command` 或 `--fail-fast` 时，先执行 benchmark suite 并写入本地 artifact，再在同一份 JSON 中补充 `benchmarkRun` 和更新后的 `benchmarkStatus`；`--fail-on-command` 在执行的 benchmark 命令失败或超时时通过 `CommandExit` 返回非零但保留报告，`--fail-on-gaps`/`--strict` 在本轮未 ready 时通过 `CommandExit` 返回非零但保留报告，`--fail-below` 调整 scorecard 阈值并可作为 CI 门禁；命令不调用 provider、不创建 session。

`deepcli.round.v1` 顶层同时输出从全局 `nextActions` 派生的 `checklist[]`，每项包含 step、label 和 command；gate-level `checklist[]` 继续从 `gates[].nextAction` 派生，内嵌 scorecard categories 继续保留分类级 `checklist[]`。这样外部 UI 读取单份 round 报告即可渲染全局动作队列、gate 修复按钮和分类修复面板，不需要解析 report 文本或额外调用 scorecard。

## 3. 推荐目录结构

```text
deepcli/
  docs/
    ai/
      REQUIREMENTS.md
      TECHNICAL_PLAN.md
  .deepcli/
    config.json
    credentials/
      deepseek-credentials.json
      kimi-credentials.json
    prompts/
    skills/
    agents/
    sessions/
    logs/
  .deepignore
  .gitignore
```

实现语言确定为 Rust。源码目录建议：

```text
Cargo.toml
src/
  main.rs
  cli/
  ui/
  runtime/
  providers/
  tools/
  permissions/
  workspace/
  session/
  skills/
  prompts/
  config/
```

## 4. 技术选型建议

实现语言采用 Rust。选择 Rust 的主要原因：

- 适合构建本地优先的单二进制 CLI。
- 对权限、状态机、工具调度和会话持久化更容易做出清晰边界。
- 适合长期维护安全敏感逻辑，例如 sandbox、审批流、shell 风险识别和凭据隔离。
- 后续发布到 Homebrew、Cargo 或独立二进制都比较自然。

Rust 方向的依赖候选：

- CLI 参数解析：`clap`。
- 异步运行时：`tokio`。
- HTTP 和流式响应：`reqwest`。
- JSON 和配置：`serde`、`serde_json`。
- 错误处理：`anyhow`、`thiserror`。
- 日志和 trace：`tracing`、`tracing-subscriber`。
- TUI 和键盘事件：`ratatui`、`crossterm`。
- TUI 和 message box：确定采用 `ratatui + crossterm`；message box 采用自研输入组件，必须支持 `Shift+Enter` 换行、`Enter` 提交、Left/Right、Home/End、Delete、Backspace、Ctrl-A/Ctrl-E、Ctrl-U/Ctrl-K、真实光标渲染、bracketed paste 粘贴大段文本并插入当前光标位置、slash 命令提示/Tab 补全、running-safe 命令标记、运行中优先展示 running-safe 命令、低信息输入本地追问且避免 `waiting_user` 短回复循环拦截、Agent 运行时旁路提问、running-safe `/terminal` 同目录终端打开且 dry-run JSON 暴露可复制的 `workspaceCommand`、running-safe `/fork --current` 持久化上下文分支、运行中 read-only `/git status|diff|branch|message` 观察工作区，以及运行中 read-only `/session` inspection 和 `/session restore-backup --dry-run --json` 只读预览；运行中所有本地旁路命令的 `--output` artifact 写入、`/completion install --force`、`/git create-branch`、`/git commit`、`/session rename`、`/session export`、`/session prune-empty --force` 或真实恢复继续要求等待任务结束或先 `/stop`。
- TUI 工具区默认展示任务观察面板，聚合 session 中的计划进度、provider usage、模型/凭据/配置健康、Prompt/Skill/Agent 能力库、验收交付门禁、最新测试、最近环境证据、待审批、开放旁路问题、工具调用总数和失败工具数；支持 Overview、Result、Changes、Usage、Health、Library、Deliver、Tools、Tests、Environment、Approvals、Trace tab，通过 `Ctrl-T` 或 `Ctrl-Left/Right` 切换；AgentRuntime 交给后台任务后，TUI 通过 active session 引用从 `.deepcli/sessions/<id>` 读取同一套 monitor 数据，header 也从 active session metadata 回填真实 title/session/provider/model/state；Overview/Trace 从最近一条 `deepcli` 或 `error` chat line 提取 `last output: ok|error ...` 摘要，Result tab 通过同一数据源展示 status、summary 和输出正文片段，并提供 `/trace --limit 30`、`/status --json`、`/session history --limit 5` 快捷动作，异步任务完成和运行中本地命令完成时把 `last_event` 更新为 `action ok|failed` 或 `running command ok|failed` 摘要，避免用户只能从长聊天输出里找结果；Result tab 为长输出维护独立 `result_scroll`，在该 tab 且输入框为空时用 PageUp/PageDown、Ctrl-Home/Ctrl-End 或工具区鼠标滚轮移动输出窗口，新任务提交、运行中本地命令完成和异步任务完成时重置到最新输出；Changes tab 在 TUI 循环中按固定间隔刷新 `git status --porcelain=v1 --untracked-files=normal` 快照，展示 Git 工作区 clean/dirty、staged/unstaged/untracked 数量、变更文件列表和受行数限制的 staged/unstaged patch preview，并支持 `[/]` 切换文件、PageUp/PageDown 滚动选中文件 patch；同时从 active session 的 `.deepcli/sessions/<id>/diffs` 读取追加式 diff 记录，展示记录总数、最近变更文件、增删行摘要和 `/diff --stat`、`/diff --name-only`、`/review`、`/handoff --format pr` 快捷动作；渲染函数只读取缓存，不在 TUI 绘制路径中执行 shell；监控面板快捷命令统一建模为 `MonitorQuickAction`，在空输入框时支持 Up/Down 选择、Enter 或鼠标点击执行，带 `<name>`/`path` 等占位符或高风险预检的命令先预填到 message box，避免误执行；quick action 标题按动作类型显示 `Enter run`、`Enter edit` 或 `Enter run/edit`，避免混合动作列表误导用户；面板高度不足时，`truncate_panel_lines_with_focus` 应围绕当前 `> /...` 快捷动作截取可见窗口并保留 `[more: ...]` 提示，确保键盘选中的动作不会被顶部详情挤出视野；Usage tab 从 `provider_turn_started`/`provider_turn_completed` 审计事件汇总 provider turn 数、平均/最大耗时、tool call 数、token、请求体大小、上下文压缩次数和 prompt cache 命中率，并展示 `/usage --json`、`/trace --limit 30`、`/status --json` 快捷命令；Health tab 复用 workspace effective config 和 active session metadata 展示当前 provider/model、默认 provider、credentials file/env/API key 状态、runtime model/endpoint、项目 config 是否存在、权限模式、provider 超时和 max iterations，并展示 `/model show --json`、`/credentials status <provider> --json`、`/config validate --json`、`/selftest --json`、`/doctor --quick` 快捷命令；当当前 provider runtime 缺少 API key 或凭据解析失败时，Health quick actions 追加 `/credentials set <provider>`，走 TUI 隐藏输入框路径，不直接暴露或记录明文 API key；Library tab 复用 `PromptStore`、`SkillStore` 和 `AgentStore`，展示 prompt 总数/自定义数/内置数、项目 skill 数、子 Agent 任务数和最近条目，并展示 `/prompt list --json`、`/prompt render <name> --file path`、`/skill list --json`、`/agent list --json` 快捷命令；Tests tab 展示最近测试记录，并提供 `/test discover --json`、`/test run --json`、`/accept --json`、`/gate --json` 快捷命令；Deliver tab 复用 `SessionMonitor` 的 plan/test/environment/approval/by-the-way/failed tool 信号，生成 acceptance checklist，并以最近环境 target 为准展示 `/review`、`/test run --json`、`/accept --env-check <target> --json`、`/gate --env-check <target> --json`、`/handoff --env-check <target> --format pr` 快捷命令；Environment tab 从 `check_environment`/`setup_environment` 工具记录提取 Docker/编译器 readiness、状态和推荐动作，并以最近环境 target 为准展示 `/env check <target> --json`、`/env plan <target> --smoke --json`、`/env test <target> --json`、`/accept --env-check <target> --json`、`/gate --env-check <target> --json`、`/handoff --env-check <target> --format pr` 快捷命令；当最近证据显示未 ready、needs/missing 或推荐 setup 时，额外展示可编辑 `/setup <target> --smoke`，只预填不直接执行，避免误触安装或拉镜像；缺少证据时默认 target 为 docker；Approvals tab 在输入框为空时支持 Up/Down 选择，选中审批时 Enter 批准、`d` 拒绝，选中开放 by-the-way 问题时 Enter 打开原生回答框并保存到当前 session；输入 slash 命令或打开 resume picker 时临时切换为相应交互面板。
- Changes tab 鼠标事件在工具区内单独处理：滚轮修改 `change_patch_scroll`，左键点击当前渲染出的 `worktree files:` 条目时按 path 定位 `WorkspaceDiffSection` 并重置滚动；无 patch 的 untracked 文件只更新状态提示，不抢占快捷动作点击。
- TUI 工具区鼠标左键优先识别第一行 tab 标签，按当前渲染文本定位 `MonitorTab` 并重置快捷动作选择；打开 resume picker 或 slash 命令建议时不处理 tab 点击，因为工具区内容已切换为对应交互面板。
- Resume picker 复用同一套鼠标事件处理，左侧 session 列表滚轮调用 `ResumePicker::move_previous_by`/`move_next_by`，左键点击可见列表项只更新 selected 和 preview，不直接确认恢复；独立 `deepcli resume` picker 和 TUI 内 `/resume` picker 都使用该逻辑。
- Slash command palette 复用工具区鼠标事件处理，滚轮只移动 `selected_command`，左键点击 `matches:` 行中的候选命令时调用与 Tab 相同的补全路径写回 message box；点击补全不直接提交命令，避免误触执行。
- Tools tab 工具调用记录保持默认折叠；空输入框时 Up/Down/PageUp/PageDown/Home/End 移动 `selected_tool`，Enter 或 Ctrl-Enter 切换展开状态，工具区鼠标滚轮移动选择且不再误滚 Result tab。渲染截断复用选中行 focus 逻辑，让长工具列表中的当前选中项始终可见；鼠标点击按当前可见窗口映射到真实 tool index，避免焦点窗口滚动后误切换顶部工具。折叠列表状态直接展示 `/session tools --limit 20 --current` 和 `/session tools --failed --limit 20 --current` 两个可编辑动作，鼠标点击只预填 message box，不直接执行命令或展开工具项。展开的当前工具在列表上方渲染多行详情预览，按字符数、行数和单行宽度限流，超限时提示快捷键；`Ctrl-O` 只预填 `/session tools --limit 20 --current`，`Ctrl-F` 只预填 `/session tools --failed --limit 20 --current`，避免 TUI 被长 stderr/stdout 撑爆或误触运行本地命令。
- Approvals tab 复用工具区鼠标事件处理，滚轮只移动 `selected_approval`，左键点击当前渲染出的 approval/BTW 列表项只更新选中项和状态提示；批准、拒绝和打开 BTW 回答框仍只能通过 Enter/`d` 等显式键盘动作触发。
- 临时输入弹层复用 `MessageBox` 作为内部输入状态，使凭据输入和 BTW 回答都支持 cursor、Delete/Backspace、Home/End、Ctrl-A/E/U/K 和 bracketed paste 插入当前位置；凭据确认路径直接读取隐藏输入 buffer，不调用 message box 的提交历史，避免 API key 被保存为普通输入历史。
- TUI 消息区支持 PageUp/PageDown 和鼠标/触控板滚轮以消息条目为单位回看，Ctrl-Home/Ctrl-End 跳到最早/最新消息；恢复会话时加载完整已持久化用户/assistant 消息，并只对单条超长消息做字符截断保护；当用户提交新输入时回到底部，避免长任务历史只能看到最近输出。
- Git 操作：MVP 可先通过受控 shell 调用 `git`；后续需要更强结构化能力时评估 `git2`。
- diff 生成：优先使用 Rust diff crate；Git 仓库内可结合 `git diff`。
- 配置格式：MVP 使用 JSON；后续可增加 TOML 支持。

需要注意的 Rust 实现成本：

- message box 的 Shift+Enter 换行、运行时旁路提问、同目录打开新终端，可能需要自研 TUI/input 组件。
- 子 Agent、工具并发和文件写入冲突需要在 runtime 层做显式任务所有权和锁策略。
- provider streaming、tool calling 和 TUI 渲染需要设计稳定的事件总线，避免 UI 与 Agent 状态耦合。

## 5. 配置设计

### 5.1 配置加载顺序

1. 内置默认配置。
2. 全局用户配置，例如 `~/.deepcli/config.json`。
3. 项目配置 `.deepcli/config.json`。
4. 环境变量覆盖。
5. CLI 参数覆盖。

### 5.2 项目配置字段

建议 `.deepcli/config.json` 包含：

```json
{
  "version": 1,
  "defaultProvider": "deepseek",
  "providers": {
    "deepseek": {
      "type": "deepseek",
      "credentialsFile": ".deepcli/credentials/deepseek-credentials.json",
      "acceptanceModel": "deepseek v4 pro",
      "capabilities": ["streaming", "reasoner", "tool_calling", "json_output", "context_cache"]
    },
    "kimi": {
      "type": "kimi",
      "credentialsFile": ".deepcli/credentials/kimi-credentials.json",
      "capabilities": ["streaming", "json_output"]
    }
  },
  "permissions": {
    "defaultMode": "sandbox",
    "workspaceRead": "ask_on_first_use",
    "workspaceWrite": "sandbox_then_approval",
    "shell": "sandbox_then_approval",
    "network": "allow",
    "git": "sandbox_then_approval",
    "dangerousCommands": "double_confirm",
    "approvalPolicy": "auto_reviewer_then_user"
  },
  "sandbox": {
    "enabledByDefault": true,
    "workspaceRoot": ".",
    "allowReadWithinWorkspace": true,
    "allowNetwork": true,
    "allowSystemWrite": false,
    "allowDangerousCommands": false,
    "onMissingPermission": "request_approval"
  },
  "agent": {
    "language": "zh-CN",
    "maxSubagentDepth": 2,
    "providerTurnTimeoutSeconds": 600,
    "requirePlanForComplexTasks": true,
    "autoReviewer": true
  },
  "usage": {
    "tokenWarningThreshold": 160000
  }
}
```

凭据文件必须只保存在本地，不进入 Git。

## 6. Provider 设计

### 6.1 ProviderClient 接口

核心接口：

- `chat(request)`：非流式请求。
- `stream(request)`：流式请求。
- `countTokens(messages)`：估算或读取 token。
- `supports(capability)`：查询能力。
- `normalizeToolCall(raw)`：标准化 tool call。
- `normalizeUsage(raw)`：标准化用量。

### 6.2 DeepSeek 适配

DeepSeek 作为默认 provider，需要支持：

- 普通对话模型。
- reasoner 模型。
- 流式输出。
- tool calling。
- JSON 输出。
- 上下文缓存。
- 限流和错误重试。
- 端到端验收阶段允许使用 DeepSeek V4 Pro 作为 Agent 执行模型；具体 API model id 不在代码中硬编码，必须来自配置或凭据文件。

### 6.3 多 Provider 扩展

Provider 层只暴露统一消息、工具和结果结构。运行时不依赖具体厂商字段，避免后续扩展时影响 Agent 主循环。

实现顺序：

1. 先完整实现 DeepSeek adapter，覆盖 streaming、reasoner、tool calling、JSON 输出、上下文缓存和用量统计。
2. 同时保留 Kimi adapter 骨架，支持读取 `.deepcli/credentials/kimi-credentials.json` 并暴露 provider 元数据。
3. OpenAI、Anthropic、本地模型只预留 trait 和配置结构，MVP 不要求完整实现。

## 7. Agent Runtime 设计

### 7.1 主循环

推荐流程：

1. 接收用户目标。
2. 加载工作区上下文。
3. 判断任务复杂度。
4. 复杂任务先生成计划。
5. 选择下一步工具调用或回复用户。
6. 工具调用进入权限引擎。
7. 执行工具并记录结果。
8. 将结果回填上下文。
9. 根据结果继续执行、修复、测试或汇报。
10. 达成目标后完成会话。

### 7.2 复杂任务强制计划

满足以下条件之一时必须计划：

- 多文件修改。
- 涉及公共接口、配置、依赖、构建脚本。
- 需要 shell 或 Git 操作。
- 用户要求完整流程。
- Agent 判断任务需要超过一个工具步骤。

计划应包含：

- 将阅读哪些文件。
- 将修改哪些区域。
- 可能影响哪些调用链。
- 如何验证。
- 哪些操作需要审批。

### 7.3 子 Agent

子 Agent 只能在 `maxSubagentDepth` 内生成。每个子 Agent 必须有明确任务边界和写入范围，防止并发冲突。

## 8. 工具系统设计

### 8.1 ToolRegistry

每个工具声明：

- 名称。
- 输入 schema。
- 输出 schema。
- 风险等级。
- 所需权限。
- 是否可并发。
- 是否会写文件。
- 是否会访问网络。

### 8.2 MVP 工具

- `read_file`
- `list_files`
- `search`
- `write_file`
- `apply_patch_or_write`
- `run_shell`
- `git_status`
- `git_diff`
- `git_commit`
- `discover_tests`
- `run_tests`
- `web_search`
- `open_terminal`
- `prompt_list`
- `prompt_get`
- `prompt_render`
- `skill_list`
- `skill_generate`
- `skill_run`
- `spawn_subagent`

### 8.3 shell 风险分级

- 低风险：`ls`、`pwd`、`rg`、`sed -n`、`git status`、只读测试发现。
- 中风险：运行测试、构建、格式化、包管理器只读命令。
- 高风险：写文件、安装依赖、修改 Git 历史、删除文件。
- 禁止或二次确认：`rm -rf`、`git reset --hard`、系统目录写入。

## 9. 权限和沙箱设计

### 9.1 Sandbox 默认模式

Agent 工作默认进入 sandbox。sandbox 是工具调用前的第一层边界，权限引擎和审批流是 sandbox 缺少权限后的升级路径。

默认规则：

- 文件读取仅限用户已授权 workspace，并且必须先应用 `.deepignore` 和隐私规则。
- 文件写入仅限 workspace 内；写入前后必须记录 diff。
- shell 命令在 sandbox 内执行，禁止系统目录写入和高风险命令。
- 网络默认允许，但 provider 请求和 web 搜索必须进行隐私过滤。
- Docker 相关操作默认属于中高风险工具，允许 Agent 请求执行，但需要 sandbox 风险评估；拉取镜像、启动容器、挂载目录等操作必须记录命令和结果。
- Git 操作默认受控；查看状态和 diff 风险较低，commit 需要审批，破坏性操作必须二次确认。
- sandbox 缺少权限时，先交给 auto-reviewer 判断；auto-reviewer 无法决定或风险过高时向用户请求 approval。

### 9.2 权限模式

- `read`：只读。
- `write`：可写，但写入需要 diff 和审批策略。
- `full_control`：默认允许大多数操作，高风险仍二次确认。
- `sandbox`：默认模式。Agent 只能在 sandbox 授权边界内执行，缺少权限时进入审批流。

### 9.3 审批流程

1. 工具调用进入 `PermissionEngine`。
2. 先判断该调用是否可在 sandbox 内执行。
3. 若 sandbox 允许，执行并记录。
4. 若 sandbox 缺少权限，根据工具、路径、命令、网络目标、Git/Docker 操作计算风险。
5. 若配置允许 auto-reviewer，先由 auto-reviewer 判断。
6. auto-reviewer 可批准低风险权限升级。
7. 无法判断或高风险操作交给用户。
8. 二次确认操作必须由用户确认。
9. 审批结果写入会话和审计日志。

### 9.4 隐私保护

- 上下文收集前应用 `.deepignore`。
- 默认忽略 `.env*`、credentials、SSH key、证书、token、构建产物、依赖目录。
- 日志和会话中不能记录 API Key 明文。
- provider 请求前进行敏感内容扫描和裁剪。

## 10. Workspace 和 Context 设计

### 10.1 首次授权

首次在目录启动时记录授权：

- 目录路径。
- 授权模式。
- 授权时间。
- ignore 配置版本。

### 10.2 上下文构建

优先读取：

- `AGENTS.md`
- `README*`
- `docs/`
- Git diff。
- 用户指定文件。
- 任务相关搜索结果。

避免一次性读取整个大型仓库。默认使用按需检索和摘要。

## 11. 文件修改设计

用户要求采用直接写文件策略，但系统仍需记录 diff：

1. 读取原文件摘要和 hash。
2. 生成修改内容。
3. 权限检查。
4. 写入文件。
5. 生成 before/after diff。
6. 以追加式唯一文件写入会话 diff 记录，避免同一文件多次修改覆盖历史 diff。
7. 可选触发 auto-reviewer；review 入口应复用 Git diff 或会话 diff 记录作为审查输入。

如果目录是 Git 仓库，优先依赖 Git diff 做回滚依据；非 Git 目录需要写入前备份或会话快照。

## 12. 会话保存和恢复

### 12.1 存储内容

- 会话元数据。
- 用户消息。
- 模型消息。
- 工具调用。
- 审批记录。
- 计划节点。
- 文件 diff。
- 测试结果。
- token 消耗。
- 错误和恢复点。

### 12.2 存储格式

MVP 可使用本地 JSONL：

- `.deepcli/sessions/<session_id>/messages.jsonl`
- `.deepcli/sessions/<session_id>/tools.jsonl`
- `.deepcli/sessions/<session_id>/plan.json`
- `.deepcli/sessions/<session_id>/diffs/`
- `.deepcli/sessions/<session_id>/backups/`
- `.deepcli/sessions/<session_id>/summary.md`

后续可迁移到 SQLite。

## 13. `/` 指令设计

`CommandRouter` 负责解析和分发：

- `/status [--json] [--output path]`，展示 workspace、工具数量、token 阈值、provider turn 超时，以及 active session 或最近有记录活动 session 的 title/state/provider/model/activity/provider turns/tokens/context/request bytes/plan/next action；无 active session 时回退到最近真实会话并给出 `/resume` 提示；默认文本保持终端可读，`--json` 输出 `deepcli.status.v1`，包含 workspace、activeSession、registeredTools、sessionSource、session activity/usage/context/plan/nextActions 和原始 report，`--output` 通过 workspace path 校验写入当前格式。
- `/init [--quick|--no-env] [--probe-provider] [--provider <name>]`，复用 `/doctor --fix` 的低风险本地 scaffold，初始化 `.deepcli/`、忽略规则和配置骨架，并输出 provider、环境、测试 next actions；`--quick`/`--no-env` 跳过环境探测。
- `/usage [--json] [--output path] [session_id|--current]`，未显式指定 session 且当前 session 没有活动/审计记录时，回退到最近有活动或审计记录的会话；默认文本输出保持适合终端排查，`--json` 输出 `deepcli.usage.v1`，包含 sessionSource、session activity、providerTurns、tokens/cacheHitRate、request max/latest bytes、context compaction、diagnostics、failedTools、failedTests、summaryPreview、nextActions 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于慢响应 issue 附件、CI artifact 或外部观测面板。
- `/diagnose [--quick|--full-env] [--probe-provider] [--provider <name>] [--limit n] [--json] [--output path] [--bundle dir] [session_id|--current]`，默认复用 `/doctor --quick` 生成 workspace health，再尝试用 `/session diagnose` 的选择策略附加最近可行动或有记录活动的 session 诊断；没有 session 时输出 workspace-only 诊断和 quick links，不返回硬错误；`--full-env` 才执行完整环境检查，`--probe-provider` 才触发在线 provider 探测，provider 前缀入口 `deepcli deepseek diagnose` 也映射到该命令；`/diagnose docker|compiler` 在解析层直接改写为 `/env check docker|compiler`，避免环境目标名被误当作 session id；`--json` 输出 `deepcli.diagnose.v1`，包含 mode、workspaceHealth、sessionDiagnosis、supportBundle、nextActions 和完整 report，`--output` 通过 workspace path 校验写入当前格式；`--bundle dir` 使用同一套 workspace path 校验创建脱敏支持包，写入 `manifest.json`、`issue.md`、`version.json`、`diagnose.json`、`quickstart.json`、`status.json`、`usage.json`、`trace.json`、`logs.json`、`sessions.json` 和 README，不创建 session，且除非同时指定 `--probe-provider` 不会新增 provider 调用；`version.json` 复用 `deepcli.version.v1`，`logs.json` 复用 `deepcli.logs.v1`，让支持包携带产品版本、workspace、默认 provider/model、provider turn timeout、命令数量和最近日志；`issue.md` 应整理成可粘贴到 issue/工单的反馈草稿，并内嵌版本、默认 provider 和 provider timeout 摘要；单个 artifact 失败时写入 error artifact 并在 manifest 标注，避免支持包因为缺少 session trace 或日志而整体失败。`/support [bundle-dir] [diagnose options]` 是 `/diagnose --bundle` 的快捷入口，默认 bundle dir 为 `.deepcli/support/latest`，第一个非 option 参数作为 bundle dir，其他参数按 diagnose options 透传；Rust CLI、wrapper 和 provider 前缀别名都必须识别它。
- `/doctor [shell] [--fix] [--quick|--no-env] [--probe-provider] [--provider <name>] [--json] [--output path]`
- `/doctor` 汇总 deepcli version、注册命令数、provider、权限、provider turn timeout、测试发现和环境检查，并把配置、凭据、`check_environment` 和已发现测试的结构化推荐转换成可执行 next action，例如 `deepcli quickstart`、`deepcli config validate`、`deepcli setup docker --smoke`、`deepcli setup compiler --smoke`、`deepcli env test compiler`；`/doctor shell` 与 `/health shell` 走同一 doctor JSON schema，默认补成 quick 本地安装体检，检查 PATH 中的 `deepcli` 是否解析到当前 workspace 的 `scripts/deepcli` 或 `target/debug/deepcli`、旧命令残留，以及 bash/zsh/fish completion 文件相对当前生成脚本的 missing/stale/up_to_date 状态，并把重指向、安装或刷新命令写入 next actions；`/health` 在解析层补成 `/doctor --quick`，`/doctor docker|compiler` 和 `/health docker|compiler` 在解析层直接改写为 `/env check docker|compiler`，避免环境目标名被误当作非法 doctor option；`--quick`/`--no-env` 跳过环境检查以便快速排查配置和凭据问题。默认文本保持终端可读，`--json` 输出 `deepcli.doctor.v1`，包含 version、mode、projectConfig、authorization、config、fixes、providers、providerReadiness、providerProbe、sessions、discoveredTests、shell、environment、nextActions 和原始 report；顶层 `nextActions` 只能输出可复制执行的 shell 命令，不从 report 反解析 `run \`/...\`` 说明文本；普通 doctor/health 诊断动作使用 `deepcli ...`，shell 安装动作可使用 `mkdir`、`chmod`、`ln`、`rm` 和 `deepcli completion install ... --force`；`--output` 通过 workspace path 校验写入当前格式，用于新用户安装体检、CI 预检、issue 附件和外部健康面板。
- Git 身份健康检查复用 `project.gitIdentity` 配置，生成 `GitIdentityReport` 并同时接入 `/doctor` 与 `/selftest`。实现只在 `git rev-parse --is-inside-work-tree` 成功时读取有效 `git config user.name` / `user.email` 和 local config；非 Git 目录返回 `no_git` 且不读取全局身份。JSON 输出增加 `gitIdentity` 字段，包含 status、expected、actual、local、issues 和 nextActions；文本报告展示摘要，mismatch 时把 issue 纳入 readiness，并给出仓库内 `git config user.name ...` / `git config user.email ...` 修复命令。
- `/trace [--limit n] [--json] [--output path] [session_id|--current]`，未显式指定 session 且当前 session 没有审计事件时，回退到最近有审计记录的会话；默认文本输出保持适合终端阅读，`--json` 输出 `deepcli.trace.v1`，包含 sessionSource、session metadata、limit、totalEvents、shownEvents、events、redacted payload、text line 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于慢响应 issue、工具调用审计附件和外部观测面板。
- `/logs [--list|--file name] [--limit n] [--json] [--output path]`，本地只读读取 workspace 的 `.deepcli/logs`，默认 tail 最近修改的日志文件；`--list` 只列日志文件，`--file` 选择指定文件且禁止绝对路径和 `..`；所有输出先脱敏，单行和总行数受限，避免大日志刷屏。`--json` 输出 `deepcli.logs.v1`，包含 logsDir、files、selectedFile、lines、lineCount、totalLines、truncated、nextActions 和 report；顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令；`--output` 通过 workspace path 校验写入当前格式；该命令不创建 session、不调用 provider，运行中 TUI 可直接执行，Usage/Trace 面板提供 `/logs --limit 80` 快捷动作。
- `/privacy [scan] [--json] [--output path] [--fail-on-findings] [--limit n] [--no-history]`，本地只读调用 Git 元数据和历史 blob 扫描，不通过 provider。实现上按 revision limit 读取 `git log`、`git ls-files`、`git rev-list`、`git ls-tree` 和 `git show`，对 remote URL 嵌入凭据、提交邮箱、tracked/historical 敏感路径、绝对本机用户目录路径、项目配置禁用词、GitHub/AWS/Slack/OpenAI/DeepSeek 形状 token 和私钥 marker 做分级；输出样本必须先经过脱敏和截断。`privacy.allowedEmails` 与 `privacy.allowedEmailDomains` 对 commit metadata 和 content email 扫描做精确邮箱或域名匹配，`privacy.allowedCommitEmails` 与 `privacy.allowedCommitDomains` 只在 commit metadata 阶段生效；`privacy.blockedTerms` 扫描提交标题和历史文件内容中的旧产品名、公司邮箱、作者姓名或内部代号等项目特定禁用词，命中后以 medium finding 记录并只展示 `<blocked-term>`；`privacy.allowedTerms` 把确认保留的迁移说明或测试夹具写入 `suppressedFindings`；`privacy.allowedUserPaths` 对 `first_redacted_user_path` 产出的脱敏本机用户路径做 exact 或子路径匹配；命中项写入 `suppressedFindings` 并从风险计数中排除，避免公开维护者邮箱、已知迁移旧路径或确认保留的禁用词样例造成 CI 噪声。`--json` 输出 `deepcli.privacy.scan.v1`，包含 status、git、counts、trackedSensitivePaths、findings、suppressedFindings、nextActions 和 report；`--output` 通过 workspace path 校验写入当前格式；`--fail-on-findings` 在未 suppressed 的 high/medium 风险存在时通过 `CommandExit` 返回非零但保留报告，适合开源前检查和 CI gate。
- `/help [command|all]`，默认列出所有指令；指定 command 时输出该指令的 usage、examples、notes；`all` 输出完整指令指南。`/quickstart` 无参数时复用帮助系统中的 quickstart topic，输出 start/configure/code/resume/accept/gate/handoff 的一页式路线，并作为 authorization-free 本地只读入口支持 `deepcli quickstart`、`deepcli help quickstart` 和 provider 前缀别名；`/quickstart --check|--json|--output path|--fail-on-missing` 走无 session 的 `CommandContext`，读取 project config、workspace authorization、默认 provider credential readiness、session 数、discovered tests、deepcli package version、registered command count 和 provider turn timeout，输出文本或稳定 `deepcli.quickstart.v1`，用于 TUI、CI、支持包和外部 onboarding UI；JSON 以 `version`、`config.providerTurnTimeoutSeconds`、`readiness.ready` 和 `readiness.missing` 暴露产品元数据和启动缺口，`quickstart_next_actions` 的顶层动作只返回可直接执行的 `deepcli ...` 命令，解释性 onboarding 留在 `steps` 和 `report`，避免外部 UI 或脚本解析 slash-command prose；`--fail-on-missing` 在缺少 project config、默认 provider API key 或 discovered tests 时通过 `CommandExit` 返回非零码，同时保留 stdout 和 `--output` 文件内容。
- `/recipes [topic|all] [--json] [--output path]`，在无 session 的 `CommandContext` 中从 recipe catalog 选择 start、code、debug、release、support、environment、shell、sota 工作流；别名 `/recipe`、`/playbook`、`/workflow`、`/workflows` 解析到同一实现；`product-loop`、`benchmark`、`round` 等 topic alias 归一到 `sota`，并输出产品循环命令链：`scorecard`、`round`、`round --run-benchmark`、`benchmark status/trends`、`baseline-template`、`compare` 和 `benchmark gate`；`sota` 的顶层 `nextActions` 复用 `build_round_report(...).next_actions`，再补充 baseline compare，使 recipe 入口和当前失败 gate 的直接修复动作保持一致；`sota` 的顶层 checklist 在生成 baseline 步骤时也复用 `sota_baseline_next_actions(workspace)`，默认 competitor baseline 缺失时不会提前展示必然失败的 compare 步骤；文本输出保持终端可复制，JSON 输出稳定 `deepcli.recipes.v1`，包含顶层 title、summary、checklist(step/label/command)、availableTopics、recipes(name/title/summary/commands/notes)、nextActions 和 report；`--output` 通过 workspace path 校验写入当前格式；该命令不创建 session、不调用 provider，运行中 TUI 可直接执行。
- `/scorecard [--json] [--output path] [--fail-below n]`，在无 session 的 `CommandContext` 中构建产品能力评分；别名 `/sota` 解析到同一实现；评分维度包括 command discovery、agent workflow、session continuity、verification/delivery、safety/privacy、provider/model ops、support operability 和 benchmark evidence；文本输出保持终端可读，JSON 输出稳定 `deepcli.scorecard.v1`，包含 score、maxScore、percent、tier、categories、gaps、nextActions、checklist 和 report；顶层 checklist 从全局 nextActions 派生 step/label/command，让外部 UI 不必为本轮全局动作队列自行命名；每个 category 保留完整分类级 nextActions，并提供从可执行 `deepcli ...` 动作派生的 checklist(step/label/command)，让外部 UI 不必解析 report；全局 nextActions 在存在 gaps 时只聚合 priority 修复动作、有 gap 分类动作和 SOTA 产品循环动作，没有 gaps 且状态为 ok 时只展示持续验收动作，避免 ready 报告被强分类 discovery 命令淹没；若 benchmark evidence ready 但 trends 仍需补历史或处理回归，顶层首项应复用 `round` 的直接修复命令 `deepcli round --json --run-benchmark --fail-on-command`；`--output` 通过 workspace path 校验写入当前格式；`--fail-below` 在总分低于阈值时保留完整输出并返回非零，供产品循环和 CI 门禁使用；该命令不创建 session、不调用 provider，运行中 TUI 可直接执行。
- `/round [--json] [--output path] [--run-benchmark|--run-suite] [--preset name|--presets a,b] [--fail-on-command] [--fail-on-gaps] [--fail-below n]`，在无 session 的 `CommandContext` 中构建产品迭代轮次报告；别名 `/iterate`、`/iteration` 解析到同一实现；默认复用 scorecard、benchmark status 和可选 goal readiness，文本输出保持终端可读，JSON 输出稳定 `deepcli.round.v1`，包含 scorecard 摘要、benchmarkStatus 摘要、可选 benchmarkRun、可选 goalStatus、gates、gaps、nextActions、checklist 和 report；顶层 checklist 从全局 nextActions 派生 step、label 和 command；每个 gate 保留 `nextAction`，并输出由可执行 `deepcli ...` 动作派生的 gate-level `checklist[]`；scorecard 摘要使用 `deepcli.scorecard.summary.v1`，其中 categories 保留 `id`、`status`、分数、gaps、分类级 `nextActions` 和 `checklist[]`，供 TUI、外部 UI 或脚本只读取 round 报告也能按全局、gate 或分类给出修复动作；scorecard gate 是分数阈值 gate，benchmark evidence 与 goal readiness 缺口由专属 gate 呈现，scorecard gaps 仍保留在总 gaps 中并影响整体 ready；benchmark gate summary 会直接列出有问题的 required preset 名称，帮助用户从 round 报告直接定位下一步证据；当 benchmark evidence 已 ready 时，round 额外复用本地 artifact 构建 benchmark trends 摘要，`insufficient_history` 和 `regression` 会生成 `benchmark_trends` failed gate、`benchmark_trends:` gap 和优先 next action；其中 `insufficient_history` 的 round 专属 next action 使用 `deepcli round --json --run-benchmark --fail-on-command`，让修复命令同时补 benchmark 样本并输出更新后的 round 报告；benchmark evidence 不 ready 时不生成独立 trends gate，避免缺失证据被重复标红；当 round 已 ready 时，外层 `nextActions` 在 `deepcli preflight --json`、`deepcli gate --json` 后追加 `sota_baseline_next_actions(workspace)`，默认 competitor baseline 存在时返回 compare，缺默认 baseline 且当前 artifact 可完整捕获时先返回 `baseline-template --from-current`，再返回手工 baseline template；goalStatus 使用稳定摘要 schema `deepcli.goal.status.summary.v1`，包含 sessionSource、session、blockerCount、blockers、plan 和 acceptance，不复制完整 goal contract；显式 benchmark run 选项会执行同一套 benchmark preset 逻辑，写入 `.deepcli/benchmarks/` artifact，并在 JSON 中包含 `benchmarkRun`；`--output` 通过 workspace path 校验写入当前格式；`--fail-on-command` 在执行的 benchmark 命令失败或超时时保留完整输出并返回非零，`--fail-on-gaps`/`--strict` 在本轮未 ready 时保留完整输出并返回非零，`--fail-below` 调整 scorecard 阈值，供持续产品循环、CI 和发布前产品证据门禁使用；该命令不创建 session、不调用 provider，默认不执行 shell，运行中 TUI 可直接执行。
- `/benchmark [presets|run-suite|run|record|status|gate|summary|trends|baseline-template|compare|list|show|clean|scorecard] [--json] [--output path]`，在无 session 的 `CommandContext` 中管理本地 benchmark 证据；无子命令、scorecard flags 或 `scorecard` 子命令兼容 `/scorecard`；`presets` 输出稳定 `deepcli.benchmark.presets.v1`，列出 name、aliases、summary、suite、case、command 和 timeout，不执行 shell；`run-suite [--preset name|--presets a,b]` 默认执行 cargo-test、preflight-quick、selftest 和 scorecard，逐个复用单 preset artifact 写入逻辑，并输出 `deepcli.benchmark.suite.v1`，包含 requestedPresets、通过/失败/超时计数、每个 artifact 摘要、最终 benchmarkStatus、report 和 nextActions；`--fail-fast` 在首个失败 preset 后停止后续 preset，`--fail-on-command` 在任一 preset 失败或超时时返回非零但仍保留 suite report 和已写入 artifact；`run --preset <name>`、`run --command <cmd>` 或 `run -- <cmd>` 显式执行本地 shell，默认超时 120 秒或 preset 指定超时，输出 `exitCode`、`durationMs`、`stdout/stderr` 字符数和脱敏截断样本，`--fail-on-command` 在命令失败或超时时返回非零但仍保留 artifact；`record` 写 `.deepcli/benchmarks/<timestamp>-<suite>-<case>.json`，schema 为 `deepcli.benchmark.record.v1`，包含 suite、case、notes、声明命令、record-only execution、Git status 摘要和 scorecard 摘要，不执行声明命令、不调用 provider；`status` 输出 `deepcli.benchmark.status.v1`，包含 ready、hasGaps、missing/weak/incomplete/failing/stale/ready 状态、meaningful preset 覆盖、`presetCoverage.requiredStatus`、最新有效 artifact、gaps、next actions 和 report，required preset gap 中的修复提示使用可直接执行的 `deepcli benchmark ...` 命令，且 next actions 在 artifact_count 为 0 时不展示 clean，只有已有本地 artifact 时才加入 `deepcli benchmark clean --dry-run --json`；只要必需 preset 缺失、只有 record-only、失败、超时或过期，就不得返回 ready；`status --fail-on-not-ready` 与 `gate` 在 status 不是 ready 时返回非零但仍保留报告；`summary` 聚合本地 artifact 历史并输出 `deepcli.benchmark.summary.v1`，包含总量、case 级通过率、失败/超时/记录数、耗时范围、最新 artifact 和 report；`trends [--limit n]` 聚合本地 artifact 历史并输出 `deepcli.benchmark.trends.v1`，包含 suite/case 级最新与上一次状态、statusTrend、durationTrend、durationDeltaMs 和 recent artifacts；当 artifact_count 大于 0 但所有 case 都缺 previous 样本时，status 为 `insufficient_history`，nextActions 首项为 `deepcli round --json --run-benchmark --fail-on-command`，并保留 `deepcli benchmark run-suite --json --fail-on-command` 作为底层补样本动作；`baseline-template [--name name] [--output path]` 生成可编辑的 `deepcli.benchmark.baseline.v1` JSON 模板，默认覆盖 required benchmark preset 并留下待填写的 status/durationMs，`--output` 写入可直接被 `compare` 读取的 workspace 内 baseline 文件；`compare [--baseline path] [--limit n]` 读取本地 artifact 历史和 workspace 内 baseline JSON，不执行 shell、不调用 provider，输出稳定 `deepcli.benchmark.compare.v1`，包含 baseline present/name/path、case 对比、statusComparison、durationDeltaMs、durationComparison、nextActions 和 report；baseline path 通过 workspace path 校验，禁止路径穿越；缺少 baseline 时 nextActions 先提示 `baseline-template --output`；baseline case 仍缺 status 或 durationMs 时 status 保持 incomplete，nextActions 先提示编辑该 baseline 文件后再 compare；`list` 输出 `deepcli.benchmark.list.v1`；`show latest|name` 输出单个 artifact；`clean [--dry-run|--force] [--keep n] [--older-than-days n] [--all]` 输出 `deepcli.benchmark.cleanup.v1`，默认 dry-run 预览清理候选并保留最新 20 个 artifact，只有显式 `--force` 才删除 `.deepcli/benchmarks/` 下的候选文件；所有 `--output` 通过 workspace path 校验写入当前格式，运行中 TUI 可直接执行。
- `baseline-template` 构造 payload 时应先计算目标 baseline path，再生成顶层 `status=needs_values`、可执行 `nextActions` 和人类可读 `report`；`--json` stdout 与 `--output` 文件必须完全一致，`compare` 读取该 baseline 时忽略这些顶层引导字段，只按 `cases` 中的 status/durationMs 判断是否 incomplete。
- `baseline-template --from-current` 先读取 `.deepcli/benchmarks/` 中最新 required preset artifact，按 preset 填入 case `status`、`durationMs` 和来源 notes；当 required preset 全部捕获且都有 duration 时输出 `status=ready` 和 compare-first nextActions，否则沿用 `needs_values` 并补充 `deepcli benchmark run-suite --json --fail-on-command` 与编辑提示。该路径只读取本地 artifact 和写入 workspace 内 `--output`，不执行 shell、不调用 provider。
- `sota_baseline_next_actions(workspace)` 负责 `scorecard`、`round` 和 `recipes sota` 的 baseline 导航：默认 competitor baseline 存在时只返回 compare；缺 baseline 且 `baseline-template --from-current` 可完整捕获当前 artifact 时，先返回 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再返回手工 competitor baseline template；缺 artifact 或缺 duration 时只返回手工 competitor baseline template。
- `benchmark_freshness_next_actions(report)` 负责把 aging/stale benchmark evidence 的刷新动作前置到 `scorecard`、`round` 和 `recipes sota` 顶层 `nextActions`；当 `freshness.refreshRecommended=true` 时先输出 `deepcli round --json --run-benchmark --fail-on-command`，随后再追加 scorecard 修复、preflight/gate 和 baseline 导航，保持 ready/gate 语义不变但避免使用过旧证据继续交付。
- `/selftest [--json] [--output path] [--fail-on-issues]`，作为产品安装/迁移后验收入口，在无 session 的 `CommandContext` 中聚合命令注册、required command 缺口、项目配置存在性、默认 provider 凭据、可恢复 session 数、`.deepcli/logs` 文件摘要、测试发现和支持入口 next actions；默认文本适合终端快速判断，`--json` 输出 `deepcli.selftest.v1`，包含 ready/status、commands、config、provider、sessions、logs、tests、issues、nextActions 和 report；`selftest_next_actions` 的顶层动作只返回可直接执行的 `deepcli ...`、`cargo ...` 或 `git ...` 命令，并通过 `git_identity_executable_next_actions` 把身份 mismatch 转换为 `git config ...` 命令，诊断说明继续保留在 `report`；`--output` 通过 workspace path 校验写入当前格式；`--fail-on-issues` 在命令面缺失、项目配置缺失、默认 provider API key 缺失或无可发现测试时通过 `CommandExit` 返回非零码，但仍保留 stdout 和输出文件；该命令不创建 session、不调用 provider，运行中 TUI 和 Health 面板可直接执行。
- `/preflight [--json] [--output path] [--dry-run] [--quick] [--fail-fast]`，作为提交/推送/发布前的一键本地检查入口，在无 session 的 `CommandContext` 中构建并执行检查清单；Cargo workspace 存在时加入 `cargo fmt --check` 和 `cargo clippy --all-targets -- -D warnings`，Git repo 内加入 `git diff --check`，并始终通过当前 deepcli binary 执行 `/selftest --json --fail-on-issues`、`/doctor --quick --json`、`/privacy --json --fail-on-findings` 和 `/gate --json`；默认 keep-going 收集所有失败，任一 required check failed 时通过 `CommandExit` 返回非零码但保留 stdout 和 `--output` 文件；`--dry-run` 把可执行检查标记为 planned，不运行命令，顶层 `nextActions` 输出可直接执行的 `deepcli preflight ... --json` 命令；`--quick` 跳过 clippy/gate，并把 privacy 子命令切换为 `/privacy --json --fail-on-findings --no-history`，仅用于缩短本地迭代等待，full mode 仍作为提交/发布前完整历史隐私扫描门禁；`--fail-fast` 在首个 required failure 后停止后续检查；文本输出保持终端可读，并在有已测量检查或 required failure 时展示 diagnostics 摘要；`--json` 输出 `deepcli.preflight.v1`，包含 status、mode、dryRun、failFast、counts、diagnostics、checklist、checks、nextActions 和 report；`checklist[]` 从 `checks[]` 派生 step、name、label、command、status 和 required，让 UI 不必解析 report 文本或自行命名发布检查步骤；`diagnostics` 包含 totalDurationMs、measuredChecks、slowestCheck、largestOutputCheck 和 failedRequiredChecks，用于定位慢检查、输出噪声和必须修复的失败项；`--output` 通过 workspace path 校验写入当前格式。
- `/completion [bash|zsh|fish|json|install|status] [--force] [--json] [--output path]`，作为 shell 安装和外部 UI 的命令目录入口，在无 session 的 `CommandContext` 中读取 `CommandRouter::command_names()` 与 `help_summaries()` 生成顶层命令、provider 快捷入口、环境/诊断参数和通用选项；默认输出安装说明，`bash`/`zsh`/`fish` 输出可 source 的补全脚本，`json` 输出稳定 `deepcli.completion.v1`，包含 program、version、shells、providers、install 和 commands；`install [bash|zsh|fish]` 检测或使用指定 shell，生成同一脚本并计算 allowlisted 用户 HOME 目标路径：zsh 写 `~/.zsh/completions/_deepcli`，bash 写 `~/.local/share/bash-completion/completions/deepcli`，fish 写 `~/.config/fish/completions/deepcli.fish`；install 默认 dry-run，输出 target、status、bytes 和 reload next action，只有显式 `--force` 才创建父目录并写文件，重复安装同内容返回 `up_to_date`；`install --json` 输出稳定 `deepcli.completion.install.v1`，包含 shell、targetPath、status、dryRun、force、bytes、parentCreated、nextActions 和 report；`status [bash|zsh|fish]` 读取同一目标路径并与当前生成脚本比较，输出 missing/stale/up_to_date 文本报告，`status --json` 输出稳定 `deepcli.completion.status.v1`，包含 shell、targetPath、status、installed、upToDate、expectedBytes、installedBytes、nextActions 和 report；`--output` 通过 workspace path 校验写入当前格式；不带 `--output` 的静态输出可在 workspace 授权前执行，带 `--output` 时仍不创建 session、不调用 provider；运行中 TUI 可直接执行。
- `/version [--json] [--output path]` 与 `/about [--json] [--output path]`，在无 session 的 `CommandContext` 中读取 effective config 和 workspace 元数据，输出 package version、workspace、项目配置存在性、default provider/model、provider count、provider turn timeout、registered command count 和 next actions；默认文本适合 issue/安装验收/人工排障，`--json` 输出 `deepcli.version.v1`，顶层 `nextActions` 直接由 `version_next_actions` 返回 `deepcli quickstart --check`、`deepcli doctor --quick`、`deepcli model show --json`、`deepcli support` 等可执行命令，不暴露 slash-command prose；`--output` 通过 workspace path 校验写入当前格式；该命令不创建 session、不调用 provider，`/about` 在解析层映射到同一实现。
- `/credentials status [provider] [--json] [--output path]|template <provider>|import-env <provider> [--force]|set <provider> [--stdin] [--force]|remove [provider]`
- `/credentials status` 默认文本保持终端可读，展示 provider credential file 是否存在、apiKey 是否配置、对应环境变量是否存在、模型和 endpoint；`--json` 输出 `deepcli.credentials.status.v1`，包含 providerCount、configuredProviders、missingProviders、providers[file/environment/model/endpoint/error]、nextActions 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于启动问题验收、TUI 健康面板、CI 预检和外部配置向导；输出不得包含明文 API key，只能展示 configured/missing。`/credentials set|import-env|template|remove` 缺省 provider 时使用 `CommandContext` 的 provider override 或 config.defaultProvider；`/login`、`/auth`、`/apikey`、`/key` 在解析层补成 `/credentials set`，本地写 `.deepcli/credentials/<provider>-credentials.json`；`/logout` 在解析层补成 `/credentials remove`，清除本地 credentials 文件中的 `apiKey`，保留 provider/model/endpoint 等元数据，并在对应环境变量仍存在时提示环境变量继续生效；缺交互终端时提示使用 `--stdin` 或 `import-env`，不得先调用 provider 或创建空 session。
- `/config show [--json] [--output path]|sources [--json] [--output path]|validate [--json] [--output path]|get <path> [--json] [--output path]|set <path> <json-value>`
- `/config show|sources|validate|get` 默认保持现有终端文本或值输出；`--json` 输出 `deepcli.config.inspect.v1`，包含 kind、path、payload 和原始 report：show 返回脱敏后的 effective config，sources 返回 global/project/env/provider API key 来源存在性，validate 返回 provider、agent、usage 校验摘要，get 返回指定配置值；`--output` 通过 workspace path 校验写入当前格式，用于 TUI 设置页、外部配置向导、CI 预检和 issue 附件。`/config set` 仍作为写操作走原有校验和权限路径，不混入 read-only inspect schema。
- `/timeout [show|set <seconds>|reset] [--json] [--output path]` 在命令层复用 `agent.providerTurnTimeoutSeconds` 的配置读写，不要求用户记忆完整配置路径。`show` 读取 effective config，`set` 和裸秒数写项目 `.deepcli/config.json`，`reset` 恢复默认配置；运行中会话执行写入后重新加载 `AppConfig` 并追加 `timeout_updated` audit，one-shot 入口在构造 runtime 前执行，避免产生空 session 或 provider 调用。默认文本输出应包含当前秒数、配置路径和慢响应排查 next actions；`--json` 输出 `deepcli.timeout.v1`，顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令，`--output` 使用 workspace path 校验写入当前格式。
- `/permissions [show] [--json] [--output path]|set-mode <sandbox|read|write>`
- `/permissions show` 默认保持现有 permissions JSON 文本；`--json` 输出 `deepcli.permissions.show.v1`，包含 effectiveMode、permissions、sandbox、riskPolicies、capabilities、requiresApproval、nextActions 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于 TUI 权限页、外部健康面板和运行前安全审计。`set-mode` 仍作为写操作更新 `.deepcli/config.json` 中的 `permissions.defaultMode`。
- `/model [show|list] [--json] [--output path]|set <provider> [model]|<provider> [model]`，以及 `/provider [provider] [model]`、`/use <provider> [model]`、`/switch <provider> [model]`
- `/model show|list` 默认保持现有终端文本；`/models` 和 `/providers` 在解析层补成 `/model list`，`/provider` 无参数或 option 开头时补成 `/model show`。`--json` 输出 `deepcli.model.inspect.v1`，show 包含默认 provider、当前会话 provider/model、选中 provider 的类型、模型、凭据文件/环境变量状态、endpoint、capabilities、nextActions 和原始 report，list 包含所有 provider 的同类摘要、providerCount/configuredProviders/missingProviders；顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令；`--output` 通过 workspace path 校验写入当前格式，用于 TUI 模型页、外部配置向导、CI 预检和 issue 附件。`/model set`、`/model <provider>`、`/provider <provider>`、`/use` 和 `/switch` 都走同一套 provider/model 参数校验和项目配置写入逻辑，运行中会话额外更新 session provider/model 和 audit；one-shot 入口在构造 `AgentRuntime` 前执行，避免模型切换命令创建空 session 或触发 provider 调用。
- `/goal [objective...] [--json] [--output path] [--acceptance-cmd cmd]` 在当前 session 中创建或展示目标契约；无 objective 时使用项目长期目标：完整实现当前项目文档中的全部需求，并且只有在所有验收要求通过、所有测试通过且目标达成后才可停止。契约保存为 session `goal.json`，schema 为 `deepcli.goal.v1`，包含 objective、sourceRequirements、stopConditions、acceptanceCommands、status、createdAt 和 updatedAt；同时写入守护 `plan.json`，让 TUI/状态面板能看到目标、文档覆盖、验收命令和最终确认步骤。`AgentRuntime::build_session_context` 必须把 active goal contract 注入后续 Provider 上下文，明确禁止 Agent 在目标、验收要求、验收命令和残余风险说明完成前声称结束。`/goal status [--json] [--output path]` 输出稳定 `deepcli.goal.status.v1`，检查需求来源文件是否存在、守护 plan 是否全部 completed、每条 acceptance command 是否有最新通过的 session test evidence，并列出 blockers 与 next actions；`/goal gate [--json] [--output path]` 复用同一报告，但在 ready=false 时通过 `CommandExit` 返回非零且保留 stdout/output 文件内容，供脚本和人工停止前门禁使用。`/goal show/status/gate` 是读/门禁型命令，无 active session 或当前 session 没有 goal 时应选择最近一个带 `goal.json` 的 session，并在文本和 JSON 中标注 `sessionSource=current|latest_with_goal`；`/goal` 创建、`/goal start`、`/goal clear|cancel` 仍必须绑定 active session，不允许回退写入历史会话。`--acceptance-cmd` 可追加项目特定验收命令；`--output` 继续使用 workspace path 校验。
- `/plan` 无参数时保留原有执行计划查看行为，读取当前 session `plan.json` 并展示 active plan。`/plan <rough requirement> [--json] [--output path] [--write-doc path]` 则进入需求澄清模式，不调用 Provider，基于用户的未成熟需求生成 Markdown 需求草稿和稳定 `deepcli.plan.requirements_draft.v1` JSON：包含 originalRequest、clarifyingQuestions、assumptions、functionalRequirements、acceptanceCriteria、nextActions 和 report。每个澄清问题都应提供 2-3 个互斥选项，并标记推荐选项；有当前 session 时，把澄清问题写入 side-question 队列，供 TUI/`/btw` 后续交互回答；`--write-doc` 把同一份草稿写入 workspace 内需求文档，用于 `docs/ai/` 产品流程。该命令用于把模糊想法转成可讨论需求文档，不应伪造用户未确认的关键事实。
- `/fork [session_id|--current] [--dry-run|--no-open] [--verify] [--app name] [--json] [--output path]` 复制已持久化会话上下文到新 session id。实现从当前 session、显式 session id 或当前 workspace 最近的可恢复对话上下文选择源会话；无 active session 且无 id 时复用 resume 候选过滤，跳过空会话、tool-only 或诊断型 session，避免默认 fork 到没有可继续上下文的记录；shell 中显式传 `--current` 但没有 active session 时返回带 `deepcli fork`、显式 session id 和结构化候选发现命令的可执行提示。创建新 metadata 后复制源 session 目录中除 `metadata.json` 外的持久化文件，给副本标题加 `Fork of ...`，记录 source/fork audit，并输出稳定 `deepcli.session.fork.v1`。默认通过 macOS Terminal 执行 `deepcli resume <new_id>` 打开新终端；默认 app 先采用显式 `--app`/`--terminal-app`，再读 `DEEPCLI_TERMINAL_APP`，再从 `TERM_PROGRAM` 仅推断 Terminal/iTerm2 这类已支持终端，最后回退 Terminal。终端 app 先通过非空和无控制字符校验，JSON 和 report 保留原始或推断后的 app 名称；自动 resume launcher 按 app kind 分支，Terminal 使用 `tell application "Terminal" to do script ...`，iTerm2 使用 `create window with default profile command ...`，其他 app 返回明确错误并保留 `workspaceResumeCommand` 给用户手动恢复。真实 fork 的 `terminal.workspaceResumeCommand` 使用同一份 shell-safe `cd <workspace> && deepcli resume <new_id>`，并作为顶层 `nextActions[0]` 输出，让用户或外部 UI 在 Terminal 打开失败、`--no-open` 或跨目录手动恢复时不依赖当前工作目录；`terminal.app` 和 `terminal.autoResumeSupported` 暴露给外部 UI。`--dry-run|--preview` 只返回 `status=dry_run`、`dryRun=true`、`fork=null`、`plannedFork`、`terminal.app`、`terminal.autoResumeSupported`、`terminal.wouldOpen`、`contextCopy` 和 next actions，不创建 session、不复制文件、不打开 Terminal；dry-run 的真实 fork next actions 必须带上显式、配置或自动推断的 `--app` 参数。当 `--json` 下源会话选择失败时，通过 `CommandExit` 保留同一 schema 的 `status=error` 报告，包含 `source=null`、`fork=null`、`plannedFork=null`、`contextCopy=null`、`error.code`、脱敏后的 `error.message`、可执行 `nextActions` 和 `report`，并先写入合法的 `--output` 再返回非零，避免外部 UI/脚本解析纯文本错误；shell 中 `--current` 误用的 `nextActions` 应优先输出 `deepcli fork --dry-run --json`，一般 no-source `nextActions` 应优先输出 `deepcli resume --dry-run --json` 和 `deepcli session list --all --limit 20 --json`，再保留 `deepcli sessions --all --limit 20`。`--no-open` 用于 CI、测试或只需要 fork id 的场景，会真实创建 fork 但跳过 Terminal；`--verify` 只在真实 fork 后读取源和副本的持久化 session 文件，输出 `verification.status`、`resumeReady`、workspace/provider/model 匹配状态、fork state、resume command，以及 message/tool/test/diff/backup 计数一致性，不执行 provider 或真正启动 resume。JSON 报告包含 `contextCopy`，声明复制模式为 `persisted_session_files`、`hotForkSupported=false`、源会话状态、运行中任务状态和 warning；真实 fork 的 `nextActions` 首项使用 workspace-aware 恢复命令，并保留 `deepcli resume <new_id>` 短命令作为后续动作，dry-run 的 `nextActions` 至少包含可执行的真实 fork 命令；源会话处于 running 时还应给出 `deepcli stop` 和带显式 app 的 `deepcli fork --current` 这类可直接执行命令，不把任务结束后的说明性文字放入 JSON 顶层动作。TUI 运行中也允许执行 `/fork --current` 和 `/fork --current --dry-run --json`，但当前正在运行的后台 Agent 任务不宣称可热分叉。
- `/diff [--staged] [--path path] [--stat|--name-only] [--limit n]`，普通 `/diff` 优先显示当前 Git diff；当 Git diff 不可用或为空时，读取当前会话最近保存的 diff 记录，当前会话无记录时回退到最近有 diff 记录的会话；`--staged` 不回退，保持 Git staged diff 语义；`--path` 可重复指定工作区相对路径前缀，复用与 `/verify --path` 相同的 diff 文件段过滤；`--stat` 输出文件级增删统计，`--name-only` 只列文件，`--limit` 限制完整 diff 行数或摘要条目数，避免大 diff 一次性刷屏。
- `/review [--path path]`，优先审查当前 Git diff；当 Git diff 不可用或为空时，读取当前会话最近保存的 diff 记录，当前会话无记录时回退到最近有 diff 记录的会话；`--path` 可重复指定工作区相对路径前缀，复用与 `/verify --path` 相同的 diff 文件段过滤；auto-reviewer findings 按 message 聚合计数，并保留最多 3 条脱敏示例，避免重复输出；review 解析 `diff --git`、`+++ b/...` 和 session diff 路径，只对新增危险命令报警；敏感信息检查复用全局脱敏器，但在 review 层跳过源码字段名、状态文本、测试/文档路径、测试上下文和检测器字面量引起的明显误报。
- `/accept [verify options]` 与 `/gate [verify options]` 在解析层复用 `SlashCommand::Verify`：`/accept` 归一化为 `/verify --run-tests ...`，`/gate` 归一化为 `/verify --run-tests --fail-on-blockers ...`；如果参数中已有 `--run-tests`、`--test-command[=...]` 或 `-- <command>`，不得重复注入默认测试请求；默认追加参数时要插入到 `--` 之前，避免把验收选项并入用户显式测试命令。无当前会话且无显式 session id 时，带 requested test run 的验收入口生成 workspace-only 报告，不回退历史 session。二者复用 `deepcli.verify.v1` JSON、workspace `--output` 校验、path/env scope、blocker 和 no-session one-shot 行为；wrapper 与 Rust CLI 都应支持 `deepcli accept ...`、`deepcli gate ...` 以及 provider 前缀形式。
- `/verify [--run-tests|--test-command <command>] [--env-check [docker|compiler]] [--path path] [--limit n] [--json] [--output path] [--fail-on-blockers] [session_id|--current]` 生成验收报告：读取 Git status、Git diff 或 session diff fallback，复用 auto-reviewer 对 diff 做风险摘要，读取最近测试记录，可选读取 Docker/编译器环境 readiness，并扫描待审批、开放旁路问题、失败工具、失败测试和未完成 plan step；`--path` 可重复指定工作区相对路径前缀，报告中展示 scope，并在 Git diff 或 session diff fallback 上做文件段过滤；`--run-tests` 通过 `run_tests` 工具执行自动发现的测试，`--test-command` 通过 `run_tests` 工具执行指定命令，结果直接进入本次报告并影响 blockers；`--env-check docker|compiler` 通过只读 `check_environment` 工具把环境证据纳入报告，不安装、不启动服务、不拉镜像，环境未 ready 或检查失败时进入 blockers，并输出 `/setup ... --smoke` 或 `/env plan ... --smoke --json` next action；`printf ok`、`echo ok`、`true` 等 smoke/no-op 命令只能作为工具链连通性信号，必须标记为弱测试证据并进入 blockers；若最近强测试早于当前 Git diff 文件 mtime 或 session diff 记录 mtime，则标记为过期测试证据并进入 blockers，避免改动后沿用旧测试结论；auto-reviewer high finding 进入 blockers，medium finding 进入 review warnings 和 next actions，不应默认阻断验收；输出 blockers 与 next actions。无 session 时仍可基于工作区 Git 状态输出报告，不能创建空 session；若本次报告运行了 fresh strong requested test，则无 session 仅作为 workspace-only 提示，不作为 blocker。`--json` 输出稳定的 `deepcli.verify.v1` 结构，包含 `status`、`hasBlockers`、`blockers`、`environment`、`nextActions`、`checklist`、scope、diff source 和完整文本报告；JSON 顶层 `nextActions` 从文本报告中的反引号命令提取，并归一为可执行 `deepcli ...`、`cargo ...` 或 `git ...` 命令，过滤说明性 prose 和 `<...>` 占位动作；`checklist[]` 由 `nextActions` 派生 `step`、`label` 和 `command`，标签由交付动作 helper 统一命名；`--output` 使用 `resolve_workspace_path` 校验路径并把所选格式写入 workspace 内 artifact，同时 stdout 保持原格式输出；`--fail-on-blockers` 保留完整报告内容，但在 blockers 非空时返回错误，供 CI、pre-commit 和脚本化验收使用；`--json --output ... --fail-on-blockers` 在返回非零退出码时仍必须向 stdout 和输出文件写入有效 JSON。
- `/handoff [--path path] [--limit n] [--env-check [docker|compiler]] [--format text|markdown|json|pr] [--output path] [--fail-on-blockers] [session_id|--current]` 生成交付摘要：复用 `/verify` 的 session 选择、Git diff/session diff fallback、path scope、review risk、测试证据质量、测试证据时效性、可选环境 readiness 和 blockers 规则，输出 workspace/session/Git/diff/review/tests/environment/risks/next actions。`--env-check docker|compiler` 通过只读 `check_environment` 工具把 Docker/编译器环境证据纳入交付报告，不安装、不启动服务、不拉镜像，环境未 ready 或检查失败时进入 blockers，并输出 `/setup ... --smoke` 或 `/env plan ... --smoke --json` next action。默认 text 输出保持终端可读；`--markdown`/`--format markdown` 生成通用 Markdown 报告；`--pr`/`--format pr` 将同一份证据重排为 Summary、Changes、Test Plan、Environment、Risks and Blockers、Checklist 结构的 PR 描述模板；`--json`/`--format json` 输出 `deepcli.handoff.v1`、status、hasBlockers、blockers、environment、nextActions、checklist、workspace/session/Git/scope/diff 和原始 report；JSON 顶层 `nextActions` 从文本报告中的反引号命令提取，并归一为可执行 `deepcli ...`、`cargo ...` 或 `git ...` 命令，过滤说明性 prose 和 `<...>` 占位动作；`checklist[]` 由 `nextActions` 派生 `step`、`label` 和 `command`，标签由交付动作 helper 统一命名；默认不写文件，显式 `--output` 时使用 `resolve_workspace_path` 校验路径并把所选格式写入 workspace 内文件，同时 stdout 保持原格式输出；`--fail-on-blockers` 复用 `CommandExit`，在 blockers 非空时保留所选格式的完整输出并返回非零退出码。该命令用于用户汇报、PR 描述或脚本自动化，不能创建 session，除显式 `--output` 外不能写文件或执行 commit。
- `/test [discover] [--json] [--output path]|run [--json] [--output path] [-- <command>]`
- `/test discover` 默认保持现有终端文本；`--json` 输出 `deepcli.test.inspect.v1`，包含 commandCount、发现到的 commands/source/sourcePath/requiresDocker/available/note、nextActions 和原始 report；`/test run` 继续通过 `run_tests` 工具执行自动发现或显式命令，显式命令在使用 inspect 选项时通过 `-- <command>` 传入，`--json` 输出同一 schema 的 run 形态，包含 passed、command、exitCode、stdout/stderr、输出长度、nextActions 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于 TUI Tests 面板、CI artifact、验收 gate 和 issue 附件。
- `/env check [docker|compiler] [--json] [--output path]|plan [docker|compiler] [--smoke] [--json] [--output path]|setup [docker|compiler] [--smoke] [--json] [--output path]|test [docker|compiler] [--json] [--output path]`
- `/env` 的所有形态都是本地 one-shot 环境入口，允许在无 active session 时执行，避免用户只做环境诊断、安装或验收就先进入 provider 对话或产生空历史；其中 check/plan 是只读预检，setup/test 继续走权限引擎和工具审计。默认文本保持终端可读，check/setup 文本在 recommended 后追加 `next:`，同时给出 `/setup <target> --smoke` 和 `/env plan <target> --smoke --json`，避免用户只看到单条推荐后不知道是否应先预览；`--json` 输出 `deepcli.env.inspect.v1`。check 形态包含 target、ready/status、checks/version/detail、recommendedAction、nextActions 和原始 report；plan 形态在 check report 基础上增加 effectiveTarget、smokeTest、wouldRun、risks、commands 和 compilerTest；setup 输出 before/after/actions/ready，test 对 compiler 使用发现到的 compiler-dev autotest，对 docker 使用 smoke setup，JSON 中必须包含 exitCode、stdout/stderr 摘要或 actions 列表、nextActions 和原始 report；所有 env JSON 顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令，避免脚本、外部 UI 或 shell 用户再解析 ``run `/...` `` 说明文本，`commands` 和 report 仍可保留 slash 命令服务 TUI。`/check [docker|compiler]` 仅在命令解析层补上 `check` action 后复用 `/env check`；`/docker`、`/compiler`、`deepcli docker` 和 `deepcli compiler` 仅在解析层补成 target-first `/env check <target>`，`/docker setup --smoke`、`/compiler test --json` 等 action 形式补成 `/env <action> <target>`；`deepcli test docker|compiler` 仅在 CLI/wrapper 入口补成 `/env test docker|compiler`，而 `deepcli test run|discover` 继续走 `/test` 项目测试；`/setup [docker|compiler]` 仅在命令解析层补上 `setup` action 后复用 `/env setup`，`/install [docker|compiler]` 仅在命令解析层补上 `install` action 后复用 `/env install`，不得绕过权限引擎、审批、输出路径校验或环境工具审计。所有形态的 `--output` 都通过 workspace path 校验，用于 TUI Env 面板、CI artifact、安装验收、issue 附件和后续 `/verify` evidence gate。
- `/git`
- `/web search <query>`，复用 `web_search` 工具和权限引擎；`/web <query>` 与 `/search <query>` 作为便捷形式；工具输出优先展示答案、摘要和来源，摘要为空时回退展示 DuckDuckGo RelatedTopics。
- `/prompt list|get <name>|render <name> [--file path] [key=value...] [--json] [--output path]|save <name> <body>|delete <name>`
- `/prompt list|get|render` 默认保持现有终端文本；`--json` 输出 `deepcli.prompt.inspect.v1`，list 包含 promptCount、prompt 名称、描述、来源、路径、正文长度和预览，get 包含完整 prompt 正文，render 包含 prompt 元数据、渲染上下文、rendered 文本和 renderedChars；顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令，有具体 prompt 时优先给出具体名称；`--output` 通过 workspace path 校验写入当前格式，用于 TUI prompt 面板、外部 prompt 管理页、issue 附件和脚本化 prompt 验收。`save/delete` 仍作为写操作走原有文件权限路径。
- `/skill list [--json] [--output path]|generate <name> <description>|run <name> [--json] [--output path]`
- `/skill list|run` 默认保持现有终端文本；`--json` 输出 `deepcli.skill.inspect.v1`，list 包含 skillCount、Skill 名称、描述、触发条件、最大深度、创建时间、metadataPath 和 instructionPath，run 包含同类元数据、instructions、instructionChars、nextActions 和原始 report；顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令，有具体 skill 时优先给出具体名称；`--output` 通过 workspace path 校验写入当前格式，用于 TUI Skill 面板、外部插件页、issue 附件和脚本化 Skill 验收。`generate` 仍作为写操作走原有文件权限路径。
- `/agent list [--json] [--output path]|show <id> [--json] [--output path]|spawn <task>`
- `/agent list|show` 默认保持现有终端文本；`--json` 输出 `deepcli.agent.inspect.v1`，list 包含 agentCount、子 Agent id/shortId、父 session、任务描述、深度、写入范围、状态、createdAt/updatedAt 和持久化路径，show 支持唯一短 id 前缀并输出单个任务详情、nextActions 和原始 report；顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令，有具体任务时优先给出短 id；`--output` 通过 workspace path 校验写入当前格式，用于 TUI Agent 面板、外部任务编排页、issue 附件和脚本化子任务诊断。`spawn` 仍作为写操作走 `spawn_subagent` 工具、权限策略和 `maxSubagentDepth` 限制。
- `/approval list [--json] [--output path] [session_id|--current] [--all]|approve <id> [--current]|deny <id> [--current]|clear [session_id|--current]`
- `/approval list` 未显式指定 session 时回退到最近有待处理审批的会话；默认文本保持终端可读，`--json` 输出 `deepcli.approval.list.v1`，包含 session metadata、activity、includeAll、pendingCount、approvals 和原始 report；`--output` 通过 workspace path 校验写入当前格式，供运行中 TUI 面板、外部审批 UI 和脚本消费；approval reason 输出前必须脱敏。`approve/deny <id>` 可跨会话定位唯一审批 id，`--current` 强制当前会话。
- `/btw ask <question>|list [--json] [--output path] [session_id|--current] [--all]|answer <id> [--current] <answer>|clear [session_id|--current]`
- `/btw list` 未显式指定 session 时回退到最近有开放旁路问题的会话；默认文本保持终端可读，`--json` 输出 `deepcli.btw.list.v1`，包含 session metadata、activity、includeAll、openCount、questions 和原始 report；`--output` 通过 workspace path 校验写入当前格式，供运行中 TUI 面板、外部旁路问题 UI 和脚本消费；question/answer 输出前必须脱敏。`answer <id>` 可跨会话定位唯一问题 id，`--current` 强制当前会话。
- `/session list [--all] [--limit n] [--json] [--output path]|search <query> [--limit n] [--json] [--output path]|next [--json] [--output path] [session_id|--current]|diagnose [--limit n] [--json] [--output path] [session_id|--current]|rename <session_id|--current> <title>|prune-empty [--dry-run|--force] [--json] [--output path]|show [--json] [--output path] [session_id|--current]|history [--limit n] [--json] [--output path] [session_id|--current]|summary [--json] [--output path] [session_id|--current]|tools [--failed] [--limit n] [--json] [--output path] [session_id|--current]|tests [--limit n] [--json] [--output path] [session_id|--current]|diffs [--limit n] [--json] [--output path] [session_id|--current]|backups [--limit n] [--json] [--output path] [session_id|--current]|restore-backup <name|latest> [--path <target>] [--session id|--current] [--dry-run] [--json] [--output path]|export [session_id|--current] [path]`；`/cleanup [sessions|empty-sessions] [--dry-run|--force] [--json] [--output path]` 是 `/session prune-empty` 的顶层易记别名。
- `/session search` 按最近更新时间遍历 session，搜索 title、summary、最近消息、工具调用、测试记录、diff 和 backup，输出匹配来源摘要，帮助用户在长任务历史中定位恢复目标；默认文本保持终端可读，`--json` 输出 `deepcli.session.search.v1`，包含 query、limit、hitCount、session metadata、match 来源、nextActions 和原始 report；有命中时 `nextActions` 围绕首个命中生成可执行的 resume preview、history、next 和 diagnose 命令，无命中时给出会话列表、resume preview 和 session list JSON；`--output` 通过 workspace path 校验写入当前格式；match 文本和 report 必须先脱敏。
- `/next [--json] [--output path] [session_id|--current]` 作为 `/session next` 的顶层快捷入口；`/session next` 读取 session metadata、activity、summary、plan、tool/test 记录、approval 和 side-question 队列，输出聚合 next actions 与 quick links。无显式 session 时优先选择最近存在待审批、开放旁路问题、失败/拒绝工具、失败测试、未完成计划或 paused/failed/waiting_user 状态的会话；若没有这些信号，再回退到最近有记录活动的会话。默认文本输出保持可读说明和 slash 语境，`--json` 输出 `deepcli.next.v1`，包含 session metadata、signals、nextActions、quickLinks 和原始 report；JSON 的 `nextActions` 与 `quickLinks` 由 session state/signals 直接生成，必须是可执行 `deepcli ...` 命令，不从文本 report 反解析自然语言；`--output` 通过 workspace path 校验写入当前格式，用于 TUI 面板、外部任务恢复 UI、脚本化 handoff 或下一 Agent 接力。
- `/session diagnose [--limit n] [--json] [--output path] [session_id|--current]` 复用 `/session next` 的会话选择策略，但输出更完整的只读诊断报告：session/activity 概览、审批/旁路/失败工具/失败测试/未完成计划的信号计数、最近失败详情、最近测试、未完成计划项、推荐 next actions 和 trace/usage/tests/tools 快捷命令；默认文本保持终端可读，`--json` 输出 `deepcli.session.diagnose.v1`，包含 session metadata、activity、signals、recentFailures、recentTests、plan、recommendedNextActions、quickLinks 和原始 report，工具 payload 需要先脱敏；JSON 的 `recommendedNextActions` 和 `quickLinks` 复用 `/session next` 的可执行 `deepcli ...` 命令生成逻辑；`--output` 通过 workspace path 校验写入当前格式；`deepcli diagnose` 和 provider 前缀下的 `deepcli deepseek diagnose` 应映射到全局 `/diagnose`，由全局诊断在存在 session 时内嵌这份 session-only 报告。
- `SessionStore::load` 统一支持完整 session id 和唯一短前缀；所有 `/session`、`/resume`、`deepcli --resume`、审批/旁路清理等手输 session id 的路径复用同一解析，歧义前缀返回明确错误。
- `/session restore-backup` 先从选定 session 找到 backup 记录；新 backup 记录包含原始 target path 时可省略 `--path`，旧记录缺少 target path 时必须显式传入；`--dry-run` 输出脱敏恢复 diff，不写文件；`--dry-run --json` 输出稳定 `deepcli.session.restore_backup.v1`，包含 session metadata、backup metadata、target path、脱敏 diff、next actions 和 report，`--output` 通过 workspace path 校验写入同一内容；真实恢复必须走 `write_file` 工具和权限引擎，并在当前 session 继续记录新的 backup/diff，真实恢复也支持同一 schema 的 `--json`/`--output` 结果。TUI Agent 运行中复用同步 dry-run 预览渲染路径，只允许不带 `--output` 的预览；其它 `/session` 子命令先经过 running read-only guard，拒绝 `rename`、`export`、`prune-empty --force` 和所有 `--output` 写入，避免在后台任务仍可能写文件时同时修改 session 状态或写 artifact。
- `/session list` 默认隐藏空 one-shot 会话，`/history` 在解析层补成 `/session list`，`--all` 展示完整列表，`--limit n` 或 `-n n` 限制长列表输出并提示已展示数量；列表和 `/session search` 命中结果都输出 `id=<短前缀>` 与 `full=<完整 UUID>`，方便复制短 id 的同时保留审计用完整 id；默认文本保持终端可读，`--json` 输出 `deepcli.session.list.v1`，包含 includeAll、limit、totalSessions、matchingSessions、shownSessions、hiddenEmptySessions、session metadata、activity、hasRecordedActivity、hasNextActionSignals 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于 resume picker、外部历史页、CI artifact 和脚本化历史审计。`/session prune-empty` 默认 dry-run，收集无 activity 且无标题的候选空会话，跳过当前会话和有标题空会话；`--force` 才删除候选目录；`--json` 输出 `deepcli.session.prune_empty.v1`，包含 dryRun、candidate/deleted/skipped counts、候选 metadata、跳过原因、next actions 和脱敏原始 report，顶层 `nextActions` 使用可直接执行的 `deepcli cleanup sessions --force`、`deepcli session list ...` 和 `deepcli history ...` 命令，`--output` 通过 workspace path 校验写入当前格式，用于外部历史页、TUI 清理确认和脚本化维护。`/session show|history|summary|tools|tests|diffs|backups|export` 未显式指定 session 时按命令类型回退到最近有对应内容的会话；`--current` 强制查看当前 session。`/session show|history|summary|tools|tests|diffs|backups` 默认文本保持终端可读，显式 `--json` 输出统一 `deepcli.session.inspect.v1`，包含 kind、session metadata、activity、payload、note 和原始 report；`--output` 通过 workspace path 校验写入当前格式，用于 TUI 面板、外部恢复 UI、CI artifact 和脚本化历史审计；message/tool/test/diff/backup 输出进入 JSON 或文本前都应经过敏感信息脱敏。
- `/session tools --failed` 过滤 `Failed`/`Denied` 工具调用，输出最近失败输入、输出/错误和诊断 next actions；未指定 session 时按 `ToolFailures` 回退到最近有失败工具的会话，而不是最近有任意工具调用的会话。
- `AgentRuntime::run_agent_task` 在首次真实任务进入 provider 前调用 session auto-title：仅当 metadata title 为空时，根据用户任务折叠空白、脱敏、截断生成标题，并同步更新 runtime/executor 的 active session；slash 命令、低信息澄清和用户手动 `/rename` 不被覆盖。
- `/session rename <session_id|--current> <title>` 直接修改选定 session metadata title；显式 session id 支持唯一短前缀，一次性命令路径不创建新的空 session。
- `/session prune-empty [--dry-run|--force] [--json] [--output path]` 默认只预览无 activity、无标题的空 session；`--force` 才删除对应 session 目录，并跳过当前 session 与有标题的空 session，避免误删用户刻意保留的记录；JSON 使用 `deepcli.session.prune_empty.v1` 暴露候选、跳过、删除数量和可执行 `deepcli ...` next actions，便于 TUI 或外部历史页先确认再执行清理。
- `/resume [session_id] [--dry-run|--preview] [--json] [--output path]`，TUI 中打开按最近活动排序的 session picker；无显式 id 时只在当前 workspace 的会话中选择候选，默认过滤空 one-shot 会话、只包含工具/测试/审计记录的诊断型 session、只包含低信息输入和 deepcli 本地澄清回复的会话，以及短小已完成的单轮任务会话。左侧选择会话并支持直接输入 filter，匹配 title、短 id、完整 id、provider 和 model，Backspace 编辑过滤条件；右侧预览 metadata、activity、summary 和最近消息，帮助用户确认恢复目标。确认恢复后，TUI 消息区从 `messages.jsonl` 读取完整已持久化用户/assistant 历史，保留滚动回看能力；启动入口 `deepcli resume` 无 id 时应在创建 `AgentRuntime` 前打开同一套 picker，确认后再恢复所选 session，避免为了选择历史对话而产生空 session、误恢复最近会话或跨项目路径恢复旧上下文。带 `--dry-run|--preview`、`--json` 或 `--output` 的 `/resume` 走本地 `CommandContext`，输出稳定 `deepcli.resume.preview.v1`，只读取持久化 session metadata、activity、summary 和最近消息，给出 resume command 与 next actions；无显式 id 时复用当前 workspace 的可恢复候选去噪规则，显式 id 仍可预览指定 session，包括旧 workspace metadata 或默认隐藏的短会话；无显式 id 且没有可恢复候选时，JSON 路径通过 `CommandExit` 保留同一 schema 的 `status=error` 报告，包含 `selected=null`、`error.code`、脱敏后的 `error.message`、可执行 `nextActions` 和 `report`，并先写入合法的 `--output` 再返回非零；顶层 `deepcli resume <id> --dry-run --json` 应在 wrapper/Rust 入口中映射到同一 slash 命令，不创建 session、不进入 TUI、不调用 provider，`--output` 继续通过 workspace path 校验。
- `/stop`，TUI 运行中中断后台 Agent task，记录 `task_stopped` audit，标记当前 session 为 `paused`，并重建可继续交互的 runtime；`/cancel` 和 `/abort` 作为别名。
- `/terminal [--dry-run|--no-open] [--app name] [--json] [--output path]` 复用 `open_terminal` 本地工具打开当前 workspace 的 macOS 终端 app；默认 app 采用同一优先级：显式 `--app`/`--terminal-app`、`DEEPCLI_TERMINAL_APP`、`TERM_PROGRAM` 推断、Terminal，其中 `TERM_PROGRAM=iTerm.app` 推断为 iTerm2，未知终端继续回退 Terminal；终端 app 拒绝空字符串和控制字符，并用 `shell_words::quote` 生成 `open -a <app> .`。dry-run 不创建进程，文本报告展示 workspace、app、命令、opened 和 next actions，JSON 使用稳定 `deepcli.terminal.v1`，包含 `app`、`command` 和 `workspaceCommand`，并支持 workspace 内 `--output` 写入。TUI 运行中 handler 必须支持 `/terminal --dry-run --json`、环境默认 app、自动推断 app 和显式 app，用于不打断后台 Agent 任务的可观测验收。

所有 running-safe 指令都应能在 Agent 运行期间安全执行；命令帮助与 slash command palette 的 `running-safe` 标记必须收敛到当前 TUI 运行中 handler 实际支持的集合，不得把本地 one-shot 但运行中不可分发的命令标成可执行。当前 TUI 运行中至少需要保留本地 `/status`、`/usage`、`/trace`、`/logs`、`/privacy`、`/fork`、`/recipes`、`/scorecard`、read-only `/round`、read-only `/benchmark` 报告子命令、read-only `/git status|diff|branch|message`、`/selftest`、`/preflight --dry-run`、`/completion`、`/approval`、read-only `/session`、`/session restore-backup --dry-run --json`、`/terminal`、`/stop`、`/quit` 与 `/btw ask/list/answer/clear`，通过当前 session 文件直接读写或按工具权限策略执行，不依赖正在后台执行的 `AgentRuntime`。`/fork` 在运行中复制当前已持久化 session 文件并提示不热复制 in-memory task。`/terminal --dry-run --json` 在运行中必须只返回报告，不创建新进程。运行中所有本地旁路命令的 `--output` artifact 写入、`/completion install --force`、`/git create-branch`、`/git commit`、`/session rename`、`/session export`、`/session prune-empty --force`、真实 restore、`/round --run-benchmark`、`/benchmark run*|record|baseline-template|clean` 和完整 `/preflight` 这类会执行 shell、写入证据/artifact、删除会话或修改 session metadata 的动作，应在 running handler 中给出明确错误，要求用户等待任务结束或先 `/stop`。by-the-way 小问题应进入旁路队列，不破坏主任务状态。

## 14. Prompt 和 Skill 设计

### 14.1 Prompt

- 内置常用 prompt。
- 用户自定义 prompt。
- 项目 prompt。
- Agent 可通过 `prompt_list`、`prompt_get` 和 `prompt_render` 发现、读取、渲染并复用 prompt；项目自定义 prompt 覆盖同名内置 prompt，删除后恢复内置默认。
- 支持变量，例如当前目录、当前分支、当前文件、当前 diff；内置变量包括 `{{workspace}}`、`{{cwd}}`、`{{branch}}`、`{{diff}}`、`{{file}}` 和 `{{file_content}}`，也支持 `/prompt render <name> key=value` 形式的自定义变量。

### 14.2 Skill

Skill 由元数据和指令文件组成：

- `skill.json`
- `SKILL.md`
- 可选脚本和模板。

Skill 调用必须受权限和最大深度限制。Agent 在使用 Skill 前应能通过 `skill_list` 发现当前项目注册的 Skill，再用 `skill_run` 读取指令。生成 Skill 时要写入说明、触发条件、输入输出、限制和测试方式。

## 15. Git 工作流设计

MVP 支持：

- 查看状态。
- 查看 diff。
- 生成 commit message。
- 本地 commit。
- 创建分支。

只读子命令 `status|diff|branch|message` 支持 `--json` 输出稳定 `deepcli.git.inspect.v1`，用于 TUI、外部 UI 和脚本读取 Git 状态而不解析纯文本。JSON 应包含 `kind`、实际执行命令、exit code、stdout/stderr、原始 raw、report 和可执行 `deepcli ...` next actions；`--output` 复用 workspace-contained 写文件校验，把当前选择的文本或 JSON 输出写入 artifact；`diff` 支持 `--staged|--cached`。只读子命令遇到未知 option 或多余参数时必须报错，避免脚本把被忽略参数后的空输出误判为结构化成功。

后续支持：

- push。
- 创建 PR。
- 处理 review comment。

危险 Git 操作必须二次确认，尤其是 `reset --hard`、强推、删除分支。

## 16. 网络和代理

网络搜索默认开启，但必须经过隐私过滤。配置支持：

- HTTP proxy。
- HTTPS proxy。
- no_proxy。
- provider endpoint override。

企业代理和私有 CA 暂不作为 MVP 重点，但配置结构应预留。

## 17. 错误处理

必须覆盖：

- API 超时。
- API 限流。
- 网络失败。
- provider 返回格式异常。
- tool call 参数错误。
- shell 命令失败。
- 测试失败。
- 文件冲突。
- 会话恢复失败。
- 上下文超限。

策略：

- API 失败使用指数退避。
- 上下文超限时摘要和裁剪。
- 工具失败时让 Agent 分析失败原因。
- 用户中断时保存恢复点。

## 18. 实施里程碑

### 18.1 Milestone 1：项目骨架和配置

- CLI 入口。
- `.deepcli/config.json` 加载。
- provider 凭据读取。
- DeepSeek 流式请求。
- 基础 REPL。
- `ratatui + crossterm` TUI 骨架和自研 message box 原型。

### 18.2 Milestone 2：Workspace 和权限

- 首次目录授权。
- `.deepignore`。
- 文件读取和搜索。
- 权限模式。
- 默认 sandbox runtime。
- sandbox 缺权限后的 approval 流程。
- 高风险命令识别。

### 18.3 Milestone 3：Agent 工具调用闭环

- ToolRegistry。
- shell、文件、Git、测试工具。
- Agent 主循环。
- 计划生成。
- 工具结果回填。

### 18.4 Milestone 4：文件修改和验证

- 直接写文件。
- diff 记录。
- 测试发现。
- 测试运行。
- 失败修复循环。

### 18.5 Milestone 5：会话和长任务

- 会话 JSONL。
- 计划状态保存。
- 中断恢复。
- token 统计和提醒。

### 18.6 Milestone 6：指令、prompt、skill

- `/` 指令系统。
- prompt 管理。
- Skill 生成、注册和调用。
- 子 Agent 深度限制。

### 18.7 Milestone 7：Git 和验收任务

- 本地 Git 工作流。
- auto-reviewer。
- 编译器项目 lv1 到 lv9+ 验收跑通。

## 19. 测试计划

- 单元测试覆盖配置、provider、权限、ignore、命令风险、会话、指令解析。
- 集成测试覆盖一次性任务、REPL、文件写入、shell 审批、测试失败修复、Git commit。
- 端到端测试使用本地样例仓库验证完整 Agent 循环。
- 最终验收必须通过调用本项目产出的 `deepcli` 产品完成，而不是手工实现 compiler。
- 最终验收使用 `work/myWork/compiler` 的需求文档和 `online-doc` 要求，由 Agent 独立生成完整 Rust 编译器实现，从 lv1 到 lv9+。
- 最终验收允许 Agent 连接 web，允许调用 DeepSeek API，并允许使用 DeepSeek V4 Pro 作为执行模型；实际 API model id 从配置读取。
- 最终验收由 Agent 独立配置 Docker 环境、拉取 image、运行自动化测试。
- 如果验收暴露 CLI 产品缺陷，应先修复 `deepcli`，再重新运行端到端验收。

## 20. 当前待确认

- DeepSeek V4 Pro 的实际 API model id 和接口能力细节。
- auto-reviewer 的判定策略和默认保守程度。
- 子 Agent 并发写入冲突处理方式。
- 非 Git 目录的备份和回滚策略。
