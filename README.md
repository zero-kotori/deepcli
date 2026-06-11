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
deepcli resume <session_id> --dry-run --json
deepcli sessions --all --limit 20
```

设置长期目标、澄清需求或复制会话：

```bash
deepcli goal "完整实现当前项目文档中的全部需求" --json
deepcli goal status --json
deepcli goal gate --json
deepcli plan "做一个可以交互式澄清需求的功能" --write-doc docs/ai/PLANNED_REQUIREMENTS.md
deepcli fork --current --dry-run --json
DEEPCLI_TERMINAL_APP=iTerm2 deepcli fork --current --dry-run --json
TERM_PROGRAM=iTerm.app deepcli fork --current --dry-run --json
deepcli fork --current --app iTerm2 --dry-run --json
deepcli fork --current --no-open --json
deepcli fork --current --no-open --verify --json
deepcli terminal --dry-run --json
DEEPCLI_TERMINAL_APP=iTerm2 deepcli terminal --dry-run --json
TERM_PROGRAM=iTerm.app deepcli terminal --dry-run --json
deepcli terminal --app iTerm2 --dry-run --json
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

准备本地环境：

```bash
deepcli env check docker --json
deepcli env plan docker --smoke --json
deepcli setup docker --smoke
deepcli env test compiler --json
```

环境命令的 JSON 顶层 `nextActions` 是可直接复制到 shell 的 `deepcli ...` 命令，例如 `deepcli setup docker --smoke` 和 `deepcli env plan docker --smoke --json`，并从这些动作派生 `checklist[]`；TUI 面向的 `commands` 和报告正文仍保留 slash 命令形式。`version/about --json`、`health/doctor --json` 也遵守同一原则：顶层动作给出可执行命令，并从可执行动作派生 `checklist[]`，配置、凭据、环境和 shell 安装说明留在 `report`、`environment` 或 `shell` 字段；`doctor shell --json` 在 PATH 或旧命令需要处理时可输出 `mkdir`、`chmod`、`rm` 等 shell 命令。

`goal` 会在当前会话中写入目标契约和守护计划，后续 Agent 上下文会持续看到验收条件，只有目标达成、要求验收通过且测试通过后才可结束；`goal status` 会检查文档来源、计划步骤和 acceptance command 的测试证据，`goal gate` 在仍有 blocker 时返回非零。无 active session 时，`goal show/status/gate` 会回退到最近一个带 goal 的会话；创建或清理 goal 仍要求 active session，避免误写历史会话。`plan` 面向不成熟需求，生成带推荐选项的澄清问题、假设、功能要求和验收标准，并可写成需求草稿。`resume --dry-run --json` 输出稳定 `deepcli.resume.preview.v1`，预览将恢复的 session、activity、summary、最近消息和下一步动作，并从可执行动作派生 `checklist[]`；无 id 时只在当前 workspace 的会话中选候选，并跳过只包含工具、测试或审计记录的诊断型 session、只包含低信息输入和本地澄清回复的会话，以及短小已完成的单轮任务会话；不进入 TUI、不创建 session、不调用 provider；`resume candidates --json` 输出稳定 `deepcli.resume.candidates.v1`，列出当前 workspace 候选、默认可恢复会话、隐藏原因和计数，解释历史是不存在还是被默认恢复过滤器隐藏；当 `--json` 下没有可恢复候选时，同一 preview schema 会输出 `status=error`、`selected=null`、`error.code`、可执行 `nextActions` 和 `checklist[]` 后返回非零，且优先引导 `deepcli resume candidates --json`。`fork` 会复制已持久化的会话上下文，默认在新 macOS Terminal 中执行 `deepcli resume <new_id>`；终端 app 优先级为 `--app`/`--terminal-app`、`DEEPCLI_TERMINAL_APP`、`TERM_PROGRAM` 自动推断、Terminal，iTerm 用户无需配置即可默认使用 iTerm2，Terminal 和 iTerm2 支持自动执行 resume，其他 app 应配合 `--no-open` 使用 JSON 中的 workspace resume 命令；TUI 内的 `/fork` 或 `/fork --current` 使用 active session，shell 中的 `deepcli fork` 无 id 时会选择当前 workspace 最近的可恢复对话上下文，并跳过空会话和诊断型 session；`--dry-run --json` 只预览源会话、复制模式、计划标题、终端 app 和下一步动作，不创建 session；源会话选择失败时同样输出 `deepcli.session.fork.v1`、`status=error`、`error.code`、可执行 `nextActions` 和 `checklist[]` 后非零退出，shell 中误用 `--current` 时优先给出 `deepcli fork --dry-run --json`，一般 no-source 动作优先给出 `deepcli resume candidates --json` 和 `deepcli session list --all --limit 20 --json`，且不会包含 `<session_id>` 这类占位命令；`--no-open` 会真实创建 fork 但跳过 Terminal；`--verify --json` 会在真实 fork 后输出 resume 健康检查，确认 workspace、provider/model 和消息/工具/测试/diff/backup 计数是否复制一致；JSON 会输出 `contextCopy`、`terminal.app`、`terminal.autoResumeSupported`、`terminal.workspaceResumeCommand`、可选 `verification`、`nextActions` 和 `checklist[]`，明确说明复制的是会话文件而不是运行中 Agent 的内存状态，并提供从任意 shell 目录都能恢复 fork 的 `cd <workspace> && deepcli resume <new_id>` 命令。源会话正在运行时，真实 fork 与 dry-run 的顶层 `nextActions` 仍只输出可执行命令，例如 `deepcli stop` 和 `deepcli fork --current`，并同步派生 checklist；运行中限制保留在 `contextCopy.warning` 与 `report`。`session restore-backup --dry-run --json` 会输出稳定 `deepcli.session.restore_backup.v1`、脱敏 diff、目标文件和下一步恢复命令，真实恢复也支持 `--json`/`--output` 并继续通过工具执行器写文件；`cleanup sessions --json` / `session prune-empty --json` 的顶层 `nextActions` 使用可直接执行的 `deepcli ...` 命令，清理说明仍留在 report；Agent 运行中仅允许 read-only `/session` 查看和不带 `--output` 的 restore-backup dry-run 预览，`rename`、`export`、`prune-empty --force`、真实恢复以及任何 `--output` artifact 写入需等待任务结束或先 `/stop`。Agent 运行中也可执行 `/fork --current` 来分支当前已落盘上下文，但不会热复制正在执行的模型任务。`terminal` 会打开当前 workspace 的新终端，终端 app 采用同一优先级，`DEEPCLI_TERMINAL_APP` 可显式设置默认 macOS 终端 app，`--app` 可单次覆盖；`deepcli.terminal.v1` 的 `nextActions` 在 dry-run、失败和真实打开成功时都只输出可执行的 `cd <workspace>` 或 `deepcli ...` 命令，使用已打开终端的说明保留在 report。

当 `resume candidates --json` 没有 eligible 候选但发现空会话时，顶层 `nextActions[0]` 会指向 `deepcli session prune-empty --dry-run --json`，让用户先安全预览清理；存在工具/诊断型隐藏会话时，还会补充 `deepcli session diagnose --limit 5 --json` 帮助解释为什么默认恢复列表为空。

`fork --dry-run --json` 在没有可分支源会话时会复用同一套候选恢复动作：空会话清理预览和诊断动作会出现在通用 `deepcli resume candidates --json` 之前，帮助用户直接处理“无法 fork 同样上下文”的原因。

`session prune-empty --dry-run --json` 会保持 JSON 工作流，顶层确认动作是 `deepcli session prune-empty --force --json`，并输出匹配的 `checklist[]`，让外部历史页或恢复面板不必自行给删除、列表和历史动作命名。

真实 fork 的顶层 `nextActions[0]` 也会使用同一条 `terminal.workspaceResumeCommand`，后续保留短形式 `deepcli resume <new_id>`，方便当前目录已正确时继续使用。

`session list --json`、`session show|history|summary|tools|tests|diffs|backups --json`、`session search --json`、`next --json` 和 `session next --json` 面向恢复 UI 和脚本输出可执行的 `deepcli ...` 动作队列，并从 `nextActions` 派生顶层 `checklist[]`；`session diagnose --json` 的 `recommendedNextActions` 和 `quickLinks` 使用同一格式，顶层 `checklist[]` 来自推荐动作；`next/session next/session diagnose` 都会从 `quickLinks` 派生 `quickLinkChecklist[]`，让恢复面板能把主操作和辅助链接分区渲染，说明性上下文保留在 `signals` 与 `report`。

`status --json`、`usage --json`、`diagnose/support --json`、support bundle `manifest.json`、`completion status/install --json`、`git status|diff|branch|message --json`、`verify/gate/handoff --json`、`config show|sources|validate|get --json`、`credentials status --json`、`permissions show --json`、`model show/list --json`、`timeout --json`、`logs --json`、`env check|plan|setup|test --json`、`test discover|run --json`、`prompt list|get|render --json`、`skill list|run --json` 和 `agent list|show --json` 也遵守同一原则：结构化 `nextActions` 只输出可直接复制执行的 `deepcli ...` 命令，不包含 `<...>` 占位动作；`status/usage` 会把 `session.nextActions` 派生成顶层 `checklist[]` 和 `session.checklist[]`，`session list/inspect`、`logs`、`config/credentials/permissions/timeout`、`env inspect`、`diagnose/support`、`completion status/install`、`git status|diff|branch|message`、`test discover/run`、`prompt list|get|render`、`skill list|run`、`agent list|show`、`model show/list` 和 support bundle manifest 也会把这些动作派生成顶层 `checklist[]`，包含 `step`、`label` 和 `command`，让恢复历史页、观测面板、设置面板、凭据向导、权限安全页、环境面板、shell 安装面板、Git 面板、测试面板、Prompt 面板、Skill 面板、Agent 面板、模型设置页和支持面板不必自行给动作命名；`verify/gate/handoff` 也会把交付动作派生成顶层 `checklist[]`；`git` 只读 JSON 使用稳定 `deepcli.git.inspect.v1`，包含执行命令、exit code、stdout/stderr、raw、report 和后续 Git/验收动作，支持 `--output` 写入 workspace 内 artifact，并会拒绝未知只读参数而不是静默忽略；`deepcli git create-branch <name>` 和 `deepcli git commit <message>` 作为受控 Git 写入口暴露在顶层帮助中，`--dry-run --json` 输出稳定 `deepcli.git.action.v1` 预览且不会执行 Git 写操作，真实执行仍走权限策略；`status.session.nextActions` 会根据会话信号给出 `deepcli next/session diagnose` 或 `deepcli usage/trace`，`usage.session.nextActions` 会给出 `deepcli trace` 和 `deepcli session diagnose`，completion 缺失或过期时会给出具体 shell 的 `deepcli completion install ... --force`，credentials 缺失或损坏时会给出具体 provider 的 `deepcli credentials set/import-env/template ...` 动作，support bundle 的人工说明保留在 manifest `notes` 中，当前已有 prompt、skill、agent 任务或测试命令时优先给出具体名称、短 id 或 shell-quoted command，说明性上下文保留在 `report`、help 或条目字段。

`approval list --json`、`approval approve|deny|clear --json`、`btw list --json` 和 `btw answer|clear --json` 也输出顶层可执行 `nextActions`，并从这些动作派生 `checklist[]`：审批队列存在 pending 项时会给出具体 approve/deny 命令；空审批或旁路队列仍给出 `--all --json` 复查和帮助入口，避免外部 UI 或脚本遇到空队列后没有下一步。

`approval approve|deny|clear --json` 输出稳定 `deepcli.approval.action.v1`，`btw answer|clear --json` 输出稳定 `deepcli.btw.action.v1`，处理后会返回 session、item 或 cleared count、nextActions 和 report；`--output` 可把结果写入 workspace 内 artifact，便于外部 UI 执行动作后立即刷新队列。

`deepcli --help` 的 Usage 区也会直接展示 approval approve/deny/clear 与 btw answer/clear，用户从顶层帮助即可发现协作队列的处理闭环。

无当前会话时，`accept` / `gate` 会使用本次 workspace 测试证据，不会被历史 session 的旧失败记录污染。

TUI 中 Agent 运行时仍可执行本地观察命令，包括 `/privacy`、`/fork`、`/terminal --dry-run`、`/recipes`、`/scorecard`、`/opportunities`、read-only `/round`、read-only `/benchmark` 报告子命令、read-only `/git status|diff|branch|message`、read-only `/session` 查看命令、`/session restore-backup --dry-run --json` 预览和 `/preflight --dry-run`；会执行 benchmark、完整 preflight、Git 写操作、shell completion 安装、session 改名/导出/强制清理、真实恢复、`--output` artifact 写入或 artifact 维护的动作需要等待当前任务结束或先 `/stop`。

任务观察面板的 quick actions 会按动作类型提示 `Enter run`、`Enter edit` 或 `Enter run/edit`，其中 `(edit)` 动作只会预填 message box。

Tools 视图中的工具调用默认折叠；折叠列表会显示可编辑的 `/session tools --limit 20 --current` 和 `/session tools --failed --limit 20 --current` 动作，鼠标点击会预填 message box，展开详情时仍可用 `Ctrl-O`/`Ctrl-F` 预填完整或失败工具输出命令。

查看任务型工作流清单：

```bash
deepcli recipes
deepcli recipes sota --json
deepcli recipes release --json
deepcli playbook support
deepcli scorecard --json
deepcli opportunities --json
deepcli opportunities --priority high --json
deepcli opportunities --effort low --json
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
deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json
deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
deepcli benchmark baselines --json
deepcli benchmark clean --dry-run --json
```

`recipes` / `playbook` 是本地只读入口，用于按 start、code、debug、release、support、environment、shell、sota 等主题查看可复制命令，不创建 session、不调用 Provider；`deepcli.recipes.v1` 顶层包含 `title`、`summary`、`nextActions` 和 `checklist`，普通单 topic 的 checklist 可直接渲染选中工作流，`recipes sota` 的 checklist 与顶层 `nextActions` 一一对应，表示当前状态感知动作队列；静态完整命令链保留在 `recipes[].commands`，说明性上下文由 `recipes[].notes` 和 report 承担；`recipes sota` 可通过 `product-loop`、`benchmark` 或 `round` alias 进入，用于串起 scorecard、round、benchmark evidence、baseline 模板、baseline compare 和 benchmark gate，并会把当前 `round` 的失败 gate 修复动作放在顶层 `nextActions` 前面，避免产品循环入口先推荐已知无效的只读报告；默认 competitor baseline 缺失且当前 benchmark artifact 可完整捕获时，顶层 `nextActions` 和 `checklist` 都会先提示 `baseline-template --from-current` 生成 compare-ready 本地基线，再保留生成 `.deepcli/baselines/competitor.json` 的手工模板动作，默认 baseline ready 后再提示执行 baseline compare；`.deepcli/baselines/` 与 `.deepcli/benchmarks/` 一样默认本地忽略。

`recipes sota --json` 也会在顶层输出非阻塞 `recommendedOpportunity`、`opportunityPriorityCounts`、`opportunityEffortCounts` 和 `opportunities[]`，复用当前 `round` 的机会对象，让产品循环入口除了按钮队列外还能展示每个机会的 summary、impact、priority、effort、status 和 checklist，并给外部 UI 一个无需扫描数组的主推荐、优先级分布和成本分布；文本模式也会在 opportunities 列表前展示推荐机会、优先级计数和成本计数，方便终端用户直接判断先做什么。

`opportunities --json` 是这些非阻塞机会的一等只读入口，输出稳定 `deepcli.opportunities.v1`，直接复用当前 `round` 的机会对象并给出顶层 `filter`、`recommendedOpportunity`、`opportunityPriorityCounts`、`opportunityEffortCounts`、`availablePriorityCounts`、`availableEffortCounts`、`nextActions` 与 `checklist[]`；`--priority high|medium|low|other` 和 `--effort high|medium|low|other` 会把机会列表、推荐机会和动作队列收敛到指定优先级或执行成本，同时保留总机会数、过滤掉的数量和全量计数；文本模式同样展示 `recommended opportunity`、`priority counts` 和 `effort counts` 摘要；每个机会都包含 `impact`、`priority` 与 `effort`，让用户或外部 UI 能同时展示收益、优先级和执行成本；ready 的 `scorecard`、`round` 和 `recipes sota` 顶层动作也会暴露 `deepcli opportunities --json`，让用户或外部 UI 不必解析完整报告即可打开机会页；当 benchmark evidence 已 ready 但 freshness 为 aging 时，机会页会把刷新 benchmark evidence 作为高优先级低成本机会放在前面；baseline 机会会先给 `deepcli benchmark baselines --json` 作为只读 inventory 检查，再给写入或 compare 动作；当当前 round 还没有可展示机会时，下一步动作会回退为 round 修复队列。

`scorecard --json` 顶层、`round --json` 顶层和 `round --json` 内嵌的 `scorecard` 摘要都会输出 `recommendedOpportunity`、`opportunityPriorityCounts`、`opportunityEffortCounts` 和 `opportunities[]`，让评分页、round 页、TUI 或外部 UI 不必扫描机会数组就能展示主推荐、优先级分布和成本分布。

`scorecard` 是本地只读产品能力评分入口，用于按命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据查看 SOTA 差距；支持稳定 `deepcli.scorecard.v1` JSON（`score` 为 raw points，`normalizedScore`/`percent` 为 0-100 展示分，并在 ok 状态输出非阻塞 `opportunities[]`）、workspace 内 `--output` 和 `--fail-below` 门禁。`scorecard.nextActions` 在存在 gaps 时会作为本轮修复队列，只聚焦直接修复、SOTA 产品循环和验收动作，避免把所有强分类探索命令混入全局列表；顶层 `scorecard.checklist[]` 会把全局 `nextActions` 结构化为 `step`、`label` 和 `command`，让 TUI、外部 UI 或脚本直接渲染本轮全局动作队列；各分类的 `nextActions` 仍会先展示本分类 gap 的修复动作，再展示通用探索动作，`categories[].checklist[]` 会从可执行 `deepcli ...` 动作派生 `step`、`label` 和 `command`，让 TUI 或外部 UI 不必解析 report 就能渲染分类级操作清单；当唯一缺口是 benchmark evidence 时，首个动作会指向可直接执行的 `deepcli round --json --run-benchmark --fail-on-command` 修复命令，不会把刚运行的 `deepcli scorecard --json` 或只读 `deepcli round --json` 再插入 benchmark evidence 修复队列，并继续露出 `deepcli recipes sota --json` 作为完整产品循环导航；当没有 gaps 且状态为 ok 时，顶层 `nextActions` 会切换为持续验收动作，不再重复输出各强分类的 discovery 命令，并按默认 competitor baseline 是否 ready、current-main 是否已 ready、当前 artifact 是否可完整捕获，在 `--from-current`、手工 baseline template 和 baseline compare 之间选择当前可执行动作；如果默认 baseline 缺失且 `.deepcli/baselines/current-main.json` 已 ready，会直接推荐手工 competitor baseline template；如果 current-main 尚未 ready 但当前 artifact 可完整捕获，会先推荐 `baseline-template --from-current` 生成 ready baseline，再保留手工 competitor baseline template；如果 benchmark evidence 已 ready 但 trend 历史仍不足或回归，scorecard 顶层动作也会优先提示 `deepcli round --json --run-benchmark --fail-on-command`，让评分入口和 round 入口指向同一个修复路径。`round` 默认聚合 scorecard、benchmark status，并在存在 goal 时纳入最近 goal readiness，输出稳定 `deepcli.round.v1`，用于每轮产品设计/工程实现后的迭代复盘、非阻塞机会发现和下一步动作判断；顶层 `round.checklist[]` 会把全局 `nextActions` 结构化为 `step`、`label` 和 `command`，让外部 UI 可以直接渲染本轮 round 动作队列；`round.gates[]` 会为每个 gate 输出 `nextAction` 和从可执行动作派生的 `checklist[]`，无动作的 gate 返回空清单，方便 UI 渲染 gate 级修复按钮；`round` 内嵌的 `scorecard.categories[]` 摘要会保留分类级 `nextActions` 和 `checklist[]`，让 TUI 或外部 UI 只读取一份 round 报告也能按分类展示修复动作；`scorecard` gate 只表示分数是否达到本轮阈值，benchmark evidence、goal readiness 和其它 gaps 会分别在专属 gate 或 gaps 列表中呈现，避免同一缺口重复标红；benchmark gate 会直接列出缺失、weak、stale、失败或超时的 required preset，用户无需再打开第二份报告才能知道该补哪些证据；当 benchmark evidence 已 ready 但 trends 仍是 `insufficient_history` 或 `regression` 时，`round` 会额外输出 `benchmark_trends` gate、对应 gap 和直接修复动作，其中单样本历史不足会优先提示 `deepcli round --json --run-benchmark --fail-on-command`，让用户补样本后立即看到新的 round 结果。`round.nextActions` 会优先给出失败 gate 的直接修复命令；当 scorecard 分数已过线且只剩 benchmark evidence 缺口时，下一步会直接指向 `deepcli round --json --run-benchmark --fail-on-command`，不再把 `deepcli scorecard --json` 或刚运行的 `deepcli round --json` 放进同一组外层动作，并保留 `deepcli recipes sota --json`；当所有 gates 通过且 round ready 时，外层动作会在 `deepcli preflight --json` 和 `deepcli gate --json` 后继续按默认 competitor baseline 是否 ready 推荐 current capture、baseline template 或 baseline compare。有未 ready 的 goal 时，JSON 会包含 `goalStatus` 摘要和 `goal_readiness` gate，并提示 `deepcli goal gate --json`。显式加 `--run-benchmark` 或 `--run-suite` 时会先执行 benchmark suite，再在同一份 round JSON 中写入 `benchmarkRun` 和更新后的 `benchmarkStatus`；`--fail-on-command` 可在 benchmark 命令失败时返回非零，`--fail-on-gaps` 可让 CI 在本轮证据、分数或 goal readiness 未 ready 时失败。`benchmark` 保留 scorecard 兼容参数，同时支持 `presets/run-suite/run/record/status/gate/summary/trends/baseline-template/compare/baselines/list/show/clean` 在 `.deepcli/benchmarks/` 下发现推荐 workload、一键执行推荐基准套件、执行单项 preset、记录、评估证据质量、门禁、汇总、趋势分析、baseline 模板、baseline 对比、baseline 清单、查看和清理稳定 `deepcli.benchmark.record.v1` / `deepcli.benchmark.suite.v1` / `deepcli.benchmark.status.v1` / `deepcli.benchmark.summary.v1` / `deepcli.benchmark.trends.v1` / `deepcli.benchmark.baseline.v1` / `deepcli.benchmark.compare.v1` / `deepcli.benchmark.baselines.v1` / `deepcli.benchmark.cleanup.v1` 证据 artifact；`run-suite` 默认执行 cargo-test、preflight-quick、selftest 和 scorecard，也可重复传入 `--preset` 指定子集；`status` 会把证据分为 missing、weak、incomplete、failing、stale 或 ready，并在 JSON 中展示 required preset 覆盖细节和 `freshness`，任一 required preset 证据超过 1 天会标为 `aging`、保留 ready 语义并把 `deepcli round --json --run-benchmark --fail-on-command` 放到刷新动作前面；gap 修复提示使用可直接执行的 `deepcli benchmark ...` 命令，避免只跑单个 cargo-test 或 smoke artifact 就被当作完整 benchmark；所有包含可执行 `nextActions` 的 benchmark JSON，包括 `presets`、`run-suite`、`run`、`record`、`status`、`summary`、`trends`、`baseline-template`、`compare`、`baselines`、`list`、`show` 和 `clean`，都会派生顶层 `checklist[]`，供 TUI、外部 UI 或脚本直接渲染 benchmark 证据动作队列；`baseline-template`、`compare` 和 `baselines` 中的人工编辑提示保留在 `nextActions` 和 `report`，不会进入 checklist；`status` 和 `summary` JSON 也包含原始 `report` 文本，方便展示同一份人类可读摘要；证据缺失时，`benchmark status.nextActions` 会优先给出 `deepcli recipes sota --json` 且不会推荐 `deepcli benchmark clean --dry-run --json`，帮助用户回到完整产品循环；证据 ready 时会在 SOTA recipe 后直接给出 `deepcli benchmark baselines --json`，让用户先查看 baseline inventory 再进入 preset 探索、执行或 gate；已有本地 artifact 时，`benchmark status.nextActions` 才会展示 dry-run clean 作为证据维护动作；`trends` 可按 suite/case 展示最近状态回归和耗时变化，当已有 artifact 但所有 case 都没有 previous 样本时会返回 `insufficient_history` 并优先提示 `deepcli round --json --run-benchmark --fail-on-command`，让用户用一个命令补样本并立即复核 round；`benchmark presets/list/summary` 和其它 trends 状态的 baseline 后续动作复用同一套默认 competitor baseline 导航，缺 baseline 时按 current-main ready 状态推荐手工 template 或 current capture，默认 baseline ready 后再推荐 compare；`baseline-template` 生成带 `status=needs_values`、`nextActions`、`checklist[]` 和 `report` 的可编辑 `deepcli.benchmark.baseline.v1` JSON，`--output` 会写入 workspace 内 baseline 文件；`compare` 只读取本地 artifact 和 workspace 内 baseline JSON，按 suite/case 输出状态回归、恢复、缺失和耗时差异，不执行 shell、不调用 Provider；`baselines` 只读取 `.deepcli/baselines/*.json`，输出每个 baseline 的 ready/needs_values/invalid 状态、默认 competitor baseline 状态和 compare/template 动作；当 baseline 仍缺 `status` 或 `durationMs` 时，`compare` 会保持 `incomplete` 并在 nextActions 中提示先编辑对应 baseline 文件；`gate` 等价于 `status --fail-on-not-ready`，便于 CI 或发布脚本在证据不足时返回非零；`clean` 默认 dry-run，可用 `--force --keep n` 或 `--older-than-days n` 删除旧本地 artifact；baseline 与 benchmark 目录默认本地忽略，不会误提交凭据或机器路径。

当 `deepcli benchmark baselines --json` 只发现 ready 的 `.deepcli/baselines/current-main.json`、但默认 `.deepcli/baselines/competitor.json` 缺失时，顶层状态会是 `needs_default`，首个动作会先生成 competitor baseline template，再保留 current-main compare 作为辅助检查。

`deepcli benchmark baselines --json` 的顶层 `summary` 会结构化展示 inventory 状态、baseline 数量、ready/needs_values/invalid 计数、默认 competitor baseline 状态、默认 baseline 是否可直接 compare、可 compare baseline 数量，以及从 checklist 派生的主推荐动作和标签；外部 baseline 页面不需要解析 `report` 或自行推导主 CTA。

`deepcli benchmark presets --json` 的顶层 `summary` 会结构化展示 preset 总数、默认 run-suite preset 数、required evidence preset 数、optional preset 数、默认 suite 动作、默认/必需 preset 名称，以及从 checklist 派生的主推荐动作和标签；每个 preset 条目也会标出 `defaultSuite` 和 `requiredEvidence`，让证据采集页不需要硬编码必跑项。

当 benchmark evidence 仍为 ready 但 `freshness.refreshRecommended=true` 时，`scorecard --json`、`round --json` 和 `recipes sota --json` 的顶层 `nextActions[0]` 会优先给出 `deepcli round --json --run-benchmark --fail-on-command`，让用户先刷新 aging/stale 证据，再继续 preflight、gate 或 baseline 对比。

`deepcli benchmark status --json` 和 `round --json` 内嵌的 `benchmarkStatus.summary` 会结构化展示证据状态、artifact/meaningful 计数、freshness 状态与年龄、required preset 覆盖、gap 数量，以及从 checklist 派生的主推荐动作和标签；benchmark evidence 页头和 round gate 详情不需要解析 `report` 或拼接多个字段才能展示刷新 CTA。

`deepcli benchmark summary --json` 的顶层 `summary` 会结构化展示历史 artifact 数、case 数、总执行数、通过/失败/超时/记录/其它计数、pass rate，以及从 checklist 派生的主推荐动作和标签；历史汇总页和外部 UI 不需要解析 `report` 或重新汇总 `cases[]` 才能展示页头指标和主 CTA。

`deepcli benchmark trends --json` 的顶层 `summary` 会结构化展示趋势状态、artifact/case 数、regression/recovered/stable pass 计数、slower/faster/flat/unknown duration 计数，以及从 checklist 派生的主推荐动作和标签；趋势页头、round gate 详情和外部 UI 不需要扫描 `trends[]` 或解析 `report` 才能展示核心趋势结论。

当 `round --json` 已 ready 且需要推进 baseline 工作流时，顶层 `nextActions` 会先给出只读 `deepcli benchmark baselines --json`，再给出 `baseline-template` 或 `compare` 动作，让外部 UI 和终端用户先看到 baseline inventory 状态，再决定是否写入本地 baseline artifact。

当 `round --json` 已 ready 时，顶层 `nextActions` 会在 `deepcli preflight --json` 和 `deepcli gate --json` 后继续给出 `deepcli recipes sota --json` 与 `deepcli opportunities --json`，让 round 主状态页既能进入完整 SOTA playbook，也能进入当前机会页。

当 `scorecard --json` 没有 gaps 且需要推进 baseline 工作流时，顶层 `nextActions` 同样先给出只读 `deepcli benchmark baselines --json`，再给出 `baseline-template` 或 `compare` 动作，让评分页、外部 UI 和终端用户不必跳到 round 才能先查看 baseline inventory。

`benchmark baseline-template --from-current` 会从最新 required benchmark artifact 预填每个 case 的 `status` 和 `durationMs`；证据完整时输出 `status=ready` 的 baseline，适合把当前版本、旧版本或手工跑完的对照版本捕获成后续 compare 可直接读取的本地基线。未传 `--output` 时只输出 JSON 预览，`nextActions` 会先提示带 `--output` 的持久化命令，不会让用户 compare 一个尚未写入的默认文件。

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
./scripts/deepcli fork --help
./scripts/deepcli help resume
./scripts/deepcli fork --current --no-open --verify --json
./scripts/deepcli terminal --dry-run --json
DEEPCLI_TERMINAL_APP=iTerm2 ./scripts/deepcli terminal --dry-run --json
TERM_PROGRAM=iTerm.app ./scripts/deepcli terminal --dry-run --json
./scripts/deepcli terminal --app iTerm2 --dry-run --json
./scripts/deepcli sessions -h
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
./scripts/deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json
./scripts/deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark baselines --json
./scripts/deepcli benchmark clean --dry-run --json
./scripts/deepcli preflight --dry-run
./scripts/deepcli release-check --dry-run
./scripts/deepcli preflight --json
```

`selftest` 和 `doctor` 会读取 `.deepcli/config.json` 中的 `project.gitIdentity`，对比当前 Git 仓库的有效 `user.name` / `user.email`，用于提交前发现错误作者身份。

`quickstart --json`、`selftest --json`、`version/about --json`、`health/doctor --json`、`status/usage --json`、`session list/inspect --json`、`config/credentials/permissions/timeout --json`、`env inspect --json`、`next/session diagnose --json`、`approval/btw --json`、`logs --json`、`prompt list|get|render --json`、`skill list|run --json`、`agent list|show --json`、`diagnose/support --json` 和 support bundle `manifest.json` 会从可执行动作派生 `checklist[]`，每项包含 `step`、`label` 和 `command`，让 TUI、外部 onboarding UI、健康面板、观测面板、恢复历史页、设置面板、凭据向导、权限安全页、环境面板、协作队列面板、Prompt 面板、Skill 面板、Agent 面板、恢复面板、支持面板、安装脚本或验收脚本可以直接渲染下一步动作；`next/session diagnose --json` 还会为辅助跳转输出 `quickLinkChecklist[]`。首次引导和诊断说明继续放在 `steps`、`report`、`environment`、`shell`、`notes` 等解释性字段中，外部 UI 或脚本不需要解析 `run \`/...\`` 文本才能推进下一步。

`preflight` / `release-check` 是提交或推送前的一键本地检查入口，会串联格式、diff whitespace、clippy、selftest、doctor、privacy 和 gate；`--dry-run` 可先预览将执行的检查且顶层 `nextActions` 给出可直接执行的 `deepcli preflight ... --json` 命令；`--quick` 可跳过较慢的 clippy/gate，并将 privacy 计划切换为 `privacy --no-history` 以加快本地迭代。提交、推送或发布前仍应运行 full preflight，因为 full mode 保留完整历史隐私扫描；JSON 顶层 `checklist[]` 会把检查队列结构化为 `step`、`label`、`command`、`status` 和 `required`，让 TUI、外部 UI 或脚本不必解析 `checks[]` 或 report 文本即可渲染发布检查清单；文本和 JSON 报告会汇总总耗时、最慢检查、最大输出检查和失败的 required check，便于快速定位发布前检查慢或噪声大的原因。

`privacy.allowedEmails` / `privacy.allowedEmailDomains` 可声明公开或允许的邮箱，让 `deepcli privacy` 将这些命中记录为 suppressed findings，而不是阻断开源前检查；只想允许提交元数据时可使用 `privacy.allowedCommitEmails` / `privacy.allowedCommitDomains`。
`privacy.blockedTerms` 可声明项目特定的禁用词，例如旧产品名、公司邮箱、作者姓名或内部代号；`privacy.allowedTerms` 可把确认保留的迁移说明或测试夹具折叠为 suppressed findings。blocked term 的样例输出会显示为 `<blocked-term>`，避免报告再次泄漏原词。
`privacy.allowedUserPaths` 可声明脱敏后的历史本机用户路径，用于折叠已知迁移遗留路径。

## 仓库

当前 GitHub 远程仓库：

```text
https://github.com/zero-kotori/deepcli
```
