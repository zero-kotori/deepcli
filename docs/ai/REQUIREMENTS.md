# deepcli 需求文档

## 1. 最终需求理解

`deepcli` 是一个面向 CLI 用户的开源 AI 编程代理工具，使用 DeepSeek API 作为核心 provider，同时预留多 provider 能力。产品目标是对齐 Claude Code、Codex CLI 一类 SOTA 编程 CLI 的核心能力，包括代码理解、自动修改、多轮执行、工具调用、审批流、安全沙箱、长任务续跑、Git/PR 工作流、Skill 生成与调用、多种 `/` 指令和完整任务循环。

项目需要形成自己的差异化能力，而不是复制现有工具源码。可参考同类产品的交互模式和用户体验，但核心架构、工具执行、权限控制、会话管理、Skill 系统、命令系统和配置系统都应由本项目独立实现。

优先支持 macOS。本地开发阶段先不考虑发布渠道，后续计划开源到 GitHub。

## 2. MVP 范围

本项目的 MVP 不是传统意义上的最小聊天 CLI，而是一个具备完整编程代理闭环的可用版本。MVP 需要覆盖以下能力：

- 混合 CLI 交互：支持一次性任务、多轮 REPL、交互式 message box 和 `/` 指令。
- DeepSeek API 接入：支持流式输出、reasoner、tool calling、JSON 输出、上下文缓存，并预留其他 provider。
- 仓库理解：首次进入目录时申请读取权限，读取当前目录上下文，支持 ignore 文件和敏感文件过滤。
- 完整 Agent 循环：`分析 -> 计划 -> 修改 -> 测试 -> 修复 -> 汇报`。
- 扩展工作流循环：`分析 -> 创建计划 -> 数据获取(如需要) -> review -> 代码实现和测试 -> review -> git 提交(可选) -> 回到计划循环`。
- 文件修改：可直接写文件，但必须结合权限配置、diff 展示和审批策略。
- 工具调用：文件读写、shell、联网、Git、测试命令、依赖命令等工具可被 Agent 调用。
- 权限控制：支持读权限、写权限、完全控制权限；高风险操作需要用户审批或二次确认。
- Sandbox 默认执行：Agent 工作默认在 sandbox 中运行；只有 sandbox 缺少权限或命中高风险策略时，才通过 auto-reviewer 或用户 approval 升级权限。
- Auto-reviewer：支持自动审批低风险或明确可判定的权限升级；无法判断时升级给用户审批。
- 会话保存与恢复：保存消息、模型回复、工具调用、计划状态、diff、测试结果和任务进度。
- Git 工作流：支持查看状态、生成 commit message、commit、分支管理，并为后续 push/PR 预留接口。
- Skill 系统：支持 Skill 生成、注册、发现、调用，受最大深度和权限约束。
- 子 Agent：支持在配置的最大深度内 spawn 子 Agent。
- `/` 指令：支持状态、上下文、token 消耗、权限、配置、会话、计划、Git、Skill、prompt 等操作。
- 便利功能：自定义 prompt 存储、内置常用 prompt、message box 支持 Shift+Enter 换行、打开相同目录的新终端、Agent 运行时 by-the-way 小问题问答。
- 测试命令发现：从 `package.json`、`pyproject.toml`、`Makefile`、`Cargo.toml` 等推断测试命令。
- token 消耗提醒：支持配置提醒阈值。
- 非 Git 目录支持：可在非 Git 目录工作；若目录是 Git 仓库，可结合 Git 做回滚和 diff 管理。

## 3. 完整版本范围

完整版本在 MVP 基础上增强以下能力：

- 多 provider 成熟支持：DeepSeek、Kimi、OpenAI、Anthropic、本地模型或兼容 OpenAI API 的服务。
- 更完整的 TUI：展示任务观察面板、计划、工具调用、diff、token/上下文消耗、模型/凭据/配置健康、Prompt/Skill/Agent 能力库、验收交付门禁、审批、旁路问题、测试状态、环境证据、日志、任务状态；任务观察区应支持 Overview、Result、Changes、Usage、Health、Library、Deliver、Tools、Tests、Environment、Approvals、Trace 等可切换视图；Agent 后台运行时也应从当前 session 文件持续读取观察数据，header 继续显示真实 session/provider/model 元数据；Overview、Trace 应展示最近一次 deepcli/error 输出摘要，让用户能立即判断刚执行的动作是否成功；Result 视图应展示最近一次 deepcli/error 输出的状态、摘要和正文片段，并提供 trace/status/history 快捷动作，避免用户只能从长聊天记录里找命令结果；Result 视图中的长输出应支持 PageUp/PageDown、Ctrl-Home/Ctrl-End 和鼠标/触控板滚轮独立回看，且新输出到达或提交新输入时自动回到最新结果；Changes 视图应展示节流刷新的 Git 工作区 dirty/clean 状态、staged/unstaged/untracked 数量、变更文件列表、受行数限制的 staged/unstaged patch 预览，并支持在 TUI 内按文件切换和 PageUp/PageDown 滚动选中文件 patch，以及当前 session 的 diff 记录数量、最近变更文件、增删行摘要和 `/diff`、`/review`、`/handoff` 快捷动作，帮助用户不用离开 TUI 就能判断本轮代码改动范围；Overview、Result、Changes、Usage、Health、Library、Deliver、Tests、Environment、Trace 等视图中的快捷命令应支持空输入框时用 Up/Down 选择、Enter 或鼠标点击直接执行，含占位符或高风险预检的命令应先预填到 message box 供用户编辑，quick action 标题应按动作类型提示 `Enter run`、`Enter edit` 或 `Enter run/edit`；终端高度较小时，面板截断应优先保留当前选中的快捷动作，而不是只保留顶部详情；Usage 视图应汇总 provider turn 数、耗时、token、请求体大小、上下文压缩和 cache 命中率，并给出 `/usage`、`/trace`、`/logs`、`/status` 快捷命令；Health 视图应展示当前 provider/model、默认 provider、API key/credentials/env 状态、配置路径、权限模式、provider 超时和快捷修复命令，API key 缺失时应直接提供 `/credentials set <provider>` 安全隐藏输入入口；Library 视图应展示内置/自定义 prompt 数、项目 skill 数、子 Agent 任务数和最近条目，并给出 `/prompt`、`/skill`、`/agent` 快捷命令；Tests 视图应展示最近测试证据，并给出 `/test discover`、`/test run`、`/accept`、`/gate` 快捷命令；Deliver 视图应汇总 plan、测试、环境、审批、旁路问题和失败工具的交付 checklist，并根据最近环境 target 给出 `/review`、`/test run`、`/accept --env-check`、`/gate --env-check`、`/handoff --env-check` 快捷命令；Environment 视图应展示最近 Docker/编译器环境检查或安装证据，并根据最近证据给出对应 target 的 `/env check`、`/env plan`、可编辑 `/setup --smoke`、`/env test`、`/accept --env-check`、`/gate --env-check`、`/handoff --env-check` 快捷命令；Approvals 视图中可直接选择、批准或拒绝待审批请求，也能为开放的 by-the-way 问题打开原生回答框并保存答案。
- Changes 视图的文件级 patch 交互应同时支持键盘 `[`/`]`、PageUp/PageDown、Ctrl-Home/Ctrl-End，以及鼠标点击变更文件列表和在工具区用鼠标/触控板滚轮滚动当前 patch。
- 任务观察区 tab 应同时支持键盘快捷键和鼠标点击切换，避免用户看到可视 tab 却只能记快捷键。
- Resume session 选择器应支持键盘、直接输入过滤、鼠标点击选择和鼠标/触控板滚轮移动选择，保持与主 TUI 工具区一致的交互能力。
- `/terminal --dry-run --json` 应输出稳定 `deepcli.terminal.v1` 报告，并包含可直接复制的 `workspaceCommand`，让用户或外部 UI 在不打开 Terminal、打开失败或非 macOS 平台时仍能手动进入同一 workspace。
- Slash 命令建议面板应支持键盘、鼠标/触控板滚轮切换候选，以及鼠标点击候选命令补全到 message box，避免用户必须记住 Tab 补全。
- Tools 视图中的工具调用应默认折叠；支持键盘 Up/Down/PageUp/PageDown/Home/End 选择、Enter 展开/折叠、鼠标点击展开/折叠、鼠标/触控板滚轮移动选择，并确保当前选中工具在面板高度不足时仍可见；折叠列表状态应直接展示完整工具输出和失败工具输出的可编辑动作，鼠标点击只预填 message box；展开当前工具时应在 TUI 内显示多行详情预览，并支持 `Ctrl-O` 预填完整工具输出命令、`Ctrl-F` 预填失败工具输出命令，避免失败工具只露出一行截断信息或让用户手输 `/session tools`。
- Approvals 视图应支持鼠标/触控板滚轮移动当前审批或 BTW 选择，并支持鼠标点击列表项选中；鼠标点击不得直接批准、拒绝或提交回答，避免误触安全敏感操作。
- 凭据输入和 by-the-way 回答等临时输入弹层应复用 message box 的光标编辑、删除、Home/End、Ctrl-A/E/U/K 和 bracketed paste 能力；凭据输入必须隐藏显示且不得进入普通消息历史或日志。
- 多 Agent 协作：planner、implementer、reviewer、tester、data collector 等角色协作。
- 长任务调度：任务暂停、恢复、失败续跑、历史查看、结构化 trace 和回放。
- GitHub 开源协作：issue/PR 上下文读取、创建 PR、review comment 处理。
- 本地向量索引和代码语义索引。
- 自动升级和配置迁移。
- 代理配置：支持企业代理、自定义 endpoint、私有网关。
- benchmark 体系：对比同类工具在完成率、耗时和成本上的表现。

## 4. 非目标

本阶段明确不做：

- 不复制 Claude Code、Codex 或其他工具的源码。
- 不实现 IDE 插件、浏览器操作、截图理解、桌面自动化。
- 不实现 MCP、Agents SDK、LangGraph 等生态协议适配。
- 不实现遥测或匿名使用统计。
- 不实现团队集中式权限管理或组织级审计。
- 不实现发布链路，包括 npm、pip、Homebrew、Cargo 或独立二进制分发。
- 不要求远程 PR 提交；当前验收只需本地 Git 仓库。

## 5. 功能清单

### 5.1 CLI 入口

- `deepcli` 命令入口。
- 在当前目录直接执行 `deepcli` 应默认启动带 message box 和任务观察面板的 TUI；旧版行式 REPL 仅作为显式 `--repl` 或 `deepcli repl` 兼容入口保留。
- 支持一次性任务参数。
- 启动 wrapper 和 Rust 二进制本体都应支持高频 slash 命令、provider 与模式的顶层别名，例如 `deepcli doctor --quick`、`deepcli version --json`、`deepcli about --json`、`deepcli health --json`、`deepcli doctor docker --json`、`deepcli diagnose`、`deepcli diagnose docker --json`、`deepcli models --json`、`deepcli providers --json`、`deepcli use kimi`、`deepcli switch deepseek deepseek-v4-pro`、`deepcli provider kimi`、`deepcli provider --json`、`deepcli history --limit 10`、`deepcli cleanup sessions --json`、`deepcli accept --json`、`deepcli gate --json`、`deepcli login deepseek`、`deepcli auth deepseek`、`deepcli apikey deepseek`、`deepcli key deepseek`、`deepcli logout deepseek`、`deepcli timeout 900`、`deepcli timeout --json`、`deepcli check docker --json`、`deepcli docker --json`、`deepcli compiler setup --smoke`、`deepcli test docker --json`、`deepcli setup docker --smoke`、`deepcli install compiler --smoke`、`deepcli init --quick`、`deepcli status`、`deepcli selftest --json`、`deepcli preflight --json`、`deepcli release-check --dry-run`、`deepcli completion zsh`、`deepcli completion json`、`deepcli trace`、`deepcli logs --limit 80`、`deepcli help doctor`、`deepcli session history --limit 20`、`deepcli sessions --all`、`deepcli stream <prompt>`、`deepcli resume [session_id]`；provider 快捷入口也应支持 `deepcli deepseek doctor --quick`、`deepcli deepseek version --json`、`deepcli deepseek about --json`、`deepcli deepseek health`、`deepcli deepseek providers`、`deepcli deepseek use`、`deepcli deepseek switch kimi`、`deepcli deepseek provider --json`、`deepcli deepseek history`、`deepcli deepseek cleanup sessions --json`、`deepcli deepseek accept --json`、`deepcli deepseek gate --json`、`deepcli deepseek login`、`deepcli deepseek logout`、`deepcli deepseek auth --stdin`、`deepcli deepseek timeout 900`、`deepcli deepseek doctor docker`、`deepcli deepseek diagnose`、`deepcli deepseek diagnose compiler`、`deepcli deepseek check docker`、`deepcli deepseek docker`、`deepcli deepseek compiler setup --smoke`、`deepcli deepseek test compiler`、`deepcli deepseek setup docker --smoke`、`deepcli deepseek help doctor`、`deepcli deepseek logs --limit 80`、`deepcli deepseek selftest --json`、`deepcli deepseek preflight --json`、`deepcli deepseek release-check --dry-run`、`deepcli deepseek completion json`、`deepcli deepseek stream <prompt>` 这类组合，不能在二进制直连时误当作普通 prompt 发给模型。
- 顶层命令别名后接 `--help` 或 `-h` 时，应在 wrapper 和 Rust 二进制本体中统一转成对应 `/help <topic>`，例如 `deepcli fork --help`、`deepcli sessions -h`、`deepcli providers --help` 和 `deepcli deepseek fork --help`；`sessions/history` 归一到 `session`，`models/providers` 归一到 `model`，不得创建 session 或调用 provider。
- 任务型工作流、产品评分、产品迭代轮次和 benchmark 证据入口也应作为本地 one-shot 支持，例如 `deepcli recipes`、`deepcli recipes release --json`、`deepcli recipes sota --json`、`deepcli recipes product-loop --json`、`deepcli playbook support`、`deepcli workflow debug`、`deepcli scorecard --json`、`deepcli round --json`、`deepcli round --json --run-benchmark --fail-on-command`、`deepcli round --json --fail-on-gaps`、`deepcli benchmark --fail-below 85`、`deepcli benchmark presets --json`、`deepcli benchmark run-suite --json --fail-on-command`、`deepcli benchmark run --preset cargo-test --json --fail-on-command`、`deepcli benchmark record --json --suite product --case scorecard`、`deepcli benchmark status --json`、`deepcli benchmark gate --json`、`deepcli benchmark list --json`、`deepcli benchmark summary --json`、`deepcli benchmark trends --json`、`deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`、`deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`、`deepcli benchmark clean --dry-run --json` 以及 provider 前缀下的 `deepcli deepseek recipes release --json`、`deepcli deepseek recipes sota --json`、`deepcli deepseek scorecard --json`、`deepcli deepseek round --json`、`deepcli deepseek round --json --run-benchmark --fail-on-command`、`deepcli deepseek benchmark run-suite --json`、`deepcli deepseek benchmark trends --json`、`deepcli deepseek benchmark baseline-template --output .deepcli/baselines/competitor.json --json`、`deepcli deepseek benchmark compare --baseline .deepcli/baselines/competitor.json --json`、`deepcli deepseek benchmark clean --dry-run --json`；这些入口只展示可复制命令清单、产品能力覆盖、产品轮次状态、benchmark 差距或本地 artifact 证据，不应创建 session 或调用 provider；`deepcli.recipes.v1` JSON 应在顶层输出 `title`、`summary` 和 `checklist[]`，其中 checklist 每项包含 step、label 和可执行 deepcli command，让外部 UI 不必解析 report 或嵌套 recipes 即可渲染选中工作流；只有显式 benchmark 执行入口和显式 `round --run-benchmark`/`--run-suite` 才会执行本地 shell。每个 scorecard category 的 `nextActions` 都应先列出当前分类 gaps 的直接修复动作，再列出通用探索和验收命令，并通过 `checklist[]` 把可执行 `deepcli ...` 动作结构化为 step、label 和 command；`scorecard.nextActions` 在存在 gaps 时应作为全局修复队列，只聚焦有 gap 分类的动作和 SOTA 产品循环命令，避免把所有 strong category 的通用探索动作混入全局列表；当唯一 gap 属于 `benchmark_evidence:` 前缀时，首项应推荐 `deepcli round --json --run-benchmark --fail-on-command` 这类可直接执行的 CLI 修复命令，并继续推荐 `deepcli recipes sota --json` 作为完整产品循环导航；同一 scorecard 报告的全局动作和 `benchmark_evidence.nextActions` 不应再包含 `deepcli scorecard --json` 自引用动作，也不应包含 `deepcli round --json` 这类只读报告动作；当没有 gaps 且状态为 ok 时，`scorecard.nextActions` 顶层列表应聚焦持续验收动作，不再混入各强分类的通用 discovery 命令，并应根据默认 competitor baseline 文件是否存在，在 `--from-current`、手工 baseline template 和 baseline compare 之间选择当前可执行动作；若 benchmark evidence 已 ready 但 trend 仍是 `insufficient_history` 或 `regression`，scorecard 顶层动作也应优先指向 `deepcli round --json --run-benchmark --fail-on-command`。`round` 的 `gates[]` 应为每个 gate 暴露 `nextAction` 和由可执行动作派生的 `checklist[]`，让 `deepcli.round.v1` 支撑 gate 级修复按钮；scorecard 摘要中每个 category 也应保留这些 `nextActions` 和 `checklist[]`，让 `deepcli.round.v1` 本身足以支撑分类级修复 UI。`round.nextActions` 应优先指向当前失败 gate 的直接修复动作；当 `scorecard` gate 已通过且剩余 scorecard gaps 都属于 `benchmark_evidence:` 前缀时，不应推荐重复运行 `deepcli scorecard --json`，而应优先推荐 `deepcli round --json --run-benchmark --fail-on-command`，并在后续动作中保留 `deepcli recipes sota --json`；当所有 gates 通过且 round ready 时，外层 `nextActions` 应在 preflight/gate 后根据默认 competitor baseline 文件是否存在选择 `--from-current`、手工 baseline template 或 baseline compare；同一 round 报告的外层 `nextActions` 不应包含自引用的 `deepcli round --json`。
- 在上一条 ready baseline 导航基础上，如果默认 competitor baseline 缺失且当前 required benchmark artifact 都能捕获 `status` 和 `durationMs`，`scorecard`、`round` 和 `recipes sota` 的顶层 `nextActions` 应先推荐 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再保留 `deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`；如果默认 competitor baseline 已存在，则只推荐 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`。
- `recipes sota --json` 的顶层 `checklist[]` 应复用同一 baseline 状态导航：默认 competitor baseline 缺失且当前 artifact 可完整捕获时展示 current capture 与手工 competitor template 步骤，不提前展示必然失败的 compare；默认 baseline 已存在时再展示 compare 步骤。静态 `recipes[].commands` 可继续作为完整参考命令链保留。
- `deepcli round --json` 顶层应输出 `checklist[]`，从全局 `nextActions` 派生 `step`、`label` 和 `command`，让 TUI、外部 UI 或脚本无需解析 report 即可渲染本轮 round 全局动作队列；`gates[].checklist[]` 和内嵌 `scorecard.categories[].checklist[]` 继续分别服务 gate 级与分类级动作。
- `scorecard --json` 顶层也应输出 `checklist[]`，从全局 `nextActions` 中可直接执行的 `deepcli ...` 动作派生 `step`、`label` 和 `command`，用于渲染本轮全局修复或持续验收队列；`categories[].checklist[]` 继续表示分类级动作。
- 当 benchmark evidence 仍为 ready 但 `freshness.refreshRecommended=true`，`scorecard`、`round` 和 `recipes sota` 的顶层 `nextActions[0]` 应优先推荐 `deepcli round --json --run-benchmark --fail-on-command`，再展示 preflight、gate、baseline template 或 baseline compare，避免用户在 aging/stale 证据上继续交付或对比。
- `ask` 和 `stream` 模式必须要求非空 prompt；`deepcli ask`、`deepcli stream`、`deepcli deepseek ask`、`deepcli kimi stream` 等缺参调用应本地报错，不得退回 TUI、创建 session 或调用 provider。
- 对明显像拼错的顶层命令或未知 CLI 命令加本地拦截，例如 `deepcli doctro --quick` 应提示 `doctor` 建议和 `deepcli ask ...` 逃生路径，不能创建 session 或调用 provider。
- 本地 one-shot slash 命令，例如 `/help`、`/version`、`/about`、`/quickstart`、`/recipes`、`/scorecard`、`/round`、`/benchmark`、`/selftest`、`/preflight`、`/completion`、`/init`、`/diagnose`、`/doctor`、`/status`、`/usage`、`/next`、`/accept`、`/gate`、`/verify`、`/handoff`、`/trace`、`/logs`、`/privacy`、`/context`、`/permissions show`、`/credentials status|template|import-env|set|remove`、`/login`、`/logout`、`/auth`、`/apikey`、`/key`、`/config show|sources|validate|get`、`/timeout [show|set|reset]`、`/model show|list|set`、`/model <provider>`、`/provider`、`/use`、`/switch`、`/prompt list|get|render`、`/skill list|run`、`/agent list|show`、`/env`/`/setup`/`/install`、`/session list|search|next|diagnose|show|history|summary|tools|tests|diffs|backups|export`、`/cleanup [sessions]` 和无 id 的 `/resume`，不应创建空会话或先调用 provider；其中 `/init` 与 `/doctor --fix` 可以创建低风险本地项目结构，`/credentials set`、`/login`、`/credentials remove`、`/logout` 等凭据写入/移除命令、`/model set` 等模型配置写入命令和 `/timeout set|reset` 等 provider 超时配置写入命令可以写入本地文件，但不能创建 session 记录或调用 provider。只读 one-shot 命令即使使用 `--yes` 临时授权，也不应为了授权本身写入 `.deepcli/authorization.json` 或污染被检查仓库。
- 支持交互式会话。
- 支持恢复历史会话；TUI 恢复后应加载完整已持久化用户/assistant 消息，并通过单条消息截断保护界面性能，而不是只显示最近固定条数。

### 5.2 Message Box

- 支持正常 IDE 常用组合键。
- `Shift+Enter` 换行，`Enter` 提交；支持 Left/Right、Home/End、Delete、Backspace、Ctrl-A/Ctrl-E、Ctrl-U/Ctrl-K 等常用行编辑快捷键，并在输入框中显示真实光标位置。
- 支持多行输入、历史输入、bracketed paste 粘贴大段文本；粘贴内容应插入当前光标位置并规范化 CRLF/CR 换行。
- 输入 `/` 时展示可筛选的命令帮助面板，支持上下选择、Tab 补全，并显示 usage、examples、注意事项和运行中可安全执行标记；Agent 运行中应优先展示 running-safe 命令，且该标记只能用于当前运行中 TUI handler 实际支持的命令，避免把本地 one-shot 但运行中不可分发的命令误标为可执行。
- 对 `1`、`ok`、`继续` 等低信息输入应先本地追问并给出 `/help`、`/status`、`/session history` 等可执行提示；追问后会话进入 `waiting_user`，用户的短回复不应再次触发同一追问循环。
- 支持 Agent 运行期间执行本地 `/status`、`/usage`、`/trace`、`/logs`、`/privacy`、`/fork`、`/recipes`、`/scorecard`、read-only `/round`、read-only `/benchmark` 报告子命令、read-only `/git status|diff|branch|message`、`/selftest`、`/preflight --dry-run`、`/completion`、`/approval`、read-only `/session`、`/session restore-backup --dry-run --json`、`/terminal`、`/stop`、`/quit` 与 by-the-way 问题记录、查看、回答和清理；运行中 `/fork` 应复制已持久化会话上下文并明确 `hotForkSupported=false`，不热复制正在运行的 Agent 任务；运行中 `/terminal --dry-run --json` 应可不创建进程输出稳定 `deepcli.terminal.v1`；运行中所有本地旁路命令的 `--output` artifact 写入、`/completion install --force`、`/git create-branch` 和 `/git commit` 应提示用户等待当前任务结束或先 `/stop`；运行中 `/session rename`、`/session export`、`/session prune-empty --force`、`/session restore-backup` 真实恢复和 `/session restore-backup --dry-run --output` 预览 artifact 写入应提示用户等待当前任务结束或先 `/stop`；`/round --run-benchmark`、`/benchmark run*|record|baseline-template|clean` 和完整 `/preflight` 这类会执行 shell、写入 benchmark 证据或维护 artifact 的动作，应提示用户等待当前任务结束或先 `/stop`；`/stop` 应中断当前任务并保留可恢复会话，`/quit` 在运行中应先停止任务再退出。
- 支持从 message box 打开相同目录的新终端；Agent 正在运行时也应作为 running-safe 本地命令立即执行，并提供 dry-run/JSON 路径供脚本和 UI 验收。
- 消息区应支持 PageUp/PageDown 或鼠标/触控板滚轮回看历史消息，Ctrl-Home/Ctrl-End 跳转到最早/最新消息，长任务中不应只能看到最后几条输出。

### 5.3 `/` 指令

建议 MVP 至少支持：

- `/help [command|all]` 与 `/quickstart [--check] [--json] [--output path] [--fail-on-missing]`：展示可用指令；支持按命令查看 usage、examples、notes，也支持输出完整指令指南；无参数 `/quickstart` 应给出启动、配置凭据、切换模型、编程、恢复会话、环境准备和 `/accept`/`/gate` 验收交付的一页式路线，且作为 authorization-free 本地只读命令不创建会话、不调用 provider；`--check`/`--json`/`--output` 应在不创建 session、不调用 provider 的前提下检查当前 workspace 的项目配置、授权、默认 provider 凭据、历史 session、测试发现、deepcli package version、注册 slash 命令数和 provider turn timeout，并输出稳定 `deepcli.quickstart.v1` schema，供 TUI、CI 和外部 onboarding UI 使用；JSON 顶层 `nextActions` 必须使用可直接执行的 `deepcli ...` 命令，`checklist[]` 必须从这些可执行动作派生 `step`、`label` 和 `command`，首次引导说明留在 `steps` 和 `report`，不得要求外部 UI 解析 `run \`/...\`` 这类 slash-command prose 或自行命名动作按钮；`--fail-on-missing` 应在缺少项目配置、默认 provider API key 或可发现测试时保留当前文本/JSON 报告并返回非零退出码，便于 CI、安装脚本和首次使用验收做 readiness gate。
- `/recipes [topic|all] [--json] [--output path]` 与别名 `/recipe`、`/playbook`、`/workflow`、`/workflows`：展示面向 start、code、debug、release、support、environment、shell、sota 的任务型命令工作流，解决 `/help all` 信息过载和 `/quickstart` 偏首次启动的问题；`sota` 主题应把 product-loop、benchmark、round 等 alias 归一到同一工作流，并串联 scorecard、round、benchmark evidence、baseline 模板、baseline compare 和 benchmark gate；不得创建 session 或调用 provider；`--json` 输出稳定 `deepcli.recipes.v1` schema，包含 availableTopics、recipes、commands、notes、nextActions 和 report；`nextActions` 必须使用可直接执行的 `deepcli ...` 命令，说明性上下文放在 notes 和 report 中；`sota` 主题的顶层 `nextActions` 应复用当前 `round` 的状态感知修复队列，并保留 baseline compare 动作，避免产品循环入口先推荐已知无效的只读报告；`--output` 必须限制在 workspace 内，供 TUI、外部 onboarding UI、团队脚本和文档生成使用。
- `/scorecard [--json] [--output path] [--fail-below n]` 与别名 `/sota`：展示产品能力覆盖和 SOTA 差距，按命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据给出分数、tier、gaps 和 next actions；不得创建 session 或调用 provider；`--json` 输出稳定 `deepcli.scorecard.v1` schema，包含 categories、score、percent、tier、gaps、nextActions、checklist 和 report；`checklist[]` 从全局 `nextActions` 派生，用于渲染本轮全局动作队列；`--output` 必须限制在 workspace 内；`--fail-below` 在分数低于阈值时保留当前文本/JSON 报告并返回非零，供产品循环、CI 或发布脚本把“接近 SOTA”变成可执行门禁。`/benchmark [presets|run-suite|run|record|status|gate|summary|trends|baseline-template|compare|list|show|clean|scorecard]` 与别名 `/bench` 保留无子命令和 scorecard flags 的兼容行为；`presets` 输出稳定 `deepcli.benchmark.presets.v1`，列出可发现的推荐 workload、命令、默认 suite/case 和 timeout，且不得执行 shell；`run-suite` 默认执行 cargo-test、preflight-quick、selftest 和 scorecard，也可重复传入 `--preset` 或逗号分隔 `--presets` 指定子集，必须输出稳定 `deepcli.benchmark.suite.v1`，包含每个 preset 的 artifact、状态、耗时、退出码、最终 benchmark status 和 next actions；`run` 只在用户显式提供 `--preset <name>`、`--command <cmd>` 或 `-- <cmd>` 时执行本地 shell，必须有默认超时、输出脱敏截断、exit code、耗时和 `--fail-on-command` 严格模式，并写入 `.deepcli/benchmarks/*.json` 的稳定 `deepcli.benchmark.record.v1` artifact；`record` 只记录 suite、case、notes、声明的 benchmark command、Git 状态摘要和 scorecard 摘要，不得隐式执行 shell；`status` 只读取本地 artifact，输出稳定 `deepcli.benchmark.status.v1`，区分 missing、weak、incomplete、failing、stale、ready，并在 `presetCoverage.requiredStatus` 中列出每个必需 preset 的状态、artifact、age 和 gap，gap 中的修复提示必须使用可直接执行的 `deepcli benchmark ...` 命令，且 smoke-only 或单个 meaningful preset 不能解除 benchmark 证据缺口；status nextActions 应包含 `deepcli recipes sota --json`，让用户能从单项证据诊断回到完整产品循环；当 artifactCount 为 0 时，status nextActions 不应包含 `deepcli benchmark clean --dry-run --json`，只有存在本地 artifact 时才展示 dry-run clean 作为证据维护动作；`status --fail-on-not-ready` 和 `gate` 在 status 不是 ready 时必须保留报告并返回非零；`status` JSON 必须包含与文本模式一致的 `report` 摘要，供 TUI、外部 UI 和脚本直接展示；`summary` 聚合本地 artifact 历史，输出稳定 `deepcli.benchmark.summary.v1`，包含总量、case 通过率、失败/超时/记录数、耗时范围、最新 artifact 和 report；`trends` 聚合本地 artifact 历史，输出稳定 `deepcli.benchmark.trends.v1`，包含 suite/case 级最近状态变化、耗时变化、回归/恢复标记和 recent artifact 列表；已有 artifact 但没有任何 case 具备 previous 样本时，`trends` 必须返回 `insufficient_history` 并在 nextActions 中优先提示 `deepcli round --json --run-benchmark --fail-on-command`，并保留 `deepcli benchmark run-suite --json --fail-on-command` 作为底层补样本动作；`status`、`summary`、`trends` 和 `compare` JSON 必须包含从可执行 `deepcli ...` nextActions 派生的顶层 `checklist[]`，每项包含 `step`、`label` 和 `command`，且不得把 compare 的人工编辑说明放进 checklist；`baseline-template [--name name] [--output path]` 输出可编辑的 `deepcli.benchmark.baseline.v1` JSON，默认包含 required benchmark preset 的 suite/case/preset/command 和待填写的 status/durationMs，`--output` 必须限制在 workspace 内并写入可被 `compare` 直接读取的 baseline 文件；`compare [--baseline path]` 输出稳定 `deepcli.benchmark.compare.v1`，只读取本地 artifact 与 workspace 内 baseline JSON，按 suite/case 展示当前与 baseline 的状态对比、缺失项、耗时 delta 和 next actions，不执行 shell、不调用 provider；当 baseline case 仍缺 status 或 durationMs 时，compare 必须保持 incomplete 并在 nextActions 中提示编辑对应 baseline 文件；`list` 输出稳定 `deepcli.benchmark.list.v1`；`show latest|name` 展示单个 artifact；`clean` 输出稳定 `deepcli.benchmark.cleanup.v1`，默认 dry-run 预览旧 artifact，支持 `--force`、`--keep n`、`--older-than-days n` 和 `--all`，不得在未显式 `--force` 时删除文件；所有 `--output` 必须限制在 workspace 内。
- `baseline-template` 的 JSON stdout 和 `--output` 文件必须使用同一份 `deepcli.benchmark.baseline.v1` payload，并包含顶层 `status=needs_values`、`nextActions` 和 `report`；`nextActions` 首先提示编辑目标 baseline 文件中的 status/durationMs，再提示运行 `deepcli benchmark compare --baseline <path> --json`，避免用户执行 round/scorecard 推荐动作后停在裸 JSON 上。
- `baseline-template --from-current` 应从最新 required benchmark artifact 捕获每个 case 的 `status` 和 `durationMs`，生成同一 schema 的 baseline；当 required cases 都有有效 artifact 和耗时时，顶层 `status` 应为 `ready`，`nextActions` 应直接提示运行 `deepcli benchmark compare --baseline <path> --json`，不得再要求手动编辑 status/durationMs；缺少 artifact 或耗时时仍保持 `needs_values` 并提示补跑 benchmark 或编辑 baseline。`scorecard`、`round` 和 `recipes sota` 的 ready 状态 nextActions 在默认 competitor baseline 缺失且当前 artifact 可完整捕获时，应先推荐 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再保留生成 `.deepcli/baselines/competitor.json` 的手工模板动作；默认 baseline 已存在时仍只推荐 baseline compare。
- `/round [--json] [--output path] [--run-benchmark|--run-suite] [--preset name|--presets a,b] [--fail-on-command] [--fail-on-gaps] [--fail-below n]` 与别名 `/iterate`、`/iteration`：展示本轮产品迭代状态；默认读取 `/scorecard`、`/benchmark status`，并在存在 goal 时读取最近 goal readiness，聚合成 ready 状态、门禁、gaps 和 next actions，不得创建 session、调用 provider 或执行 shell；`scorecard` gate 应只表示分数是否达到 `--fail-below` 阈值，benchmark evidence 和 goal readiness 缺口必须由专属 gate 呈现，其它缺口保留在 gaps 列表中，避免同一问题重复标红；benchmark evidence gate 应内联列出缺失、weak、stale、失败或超时的 required preset 摘要，让用户无需额外打开 `/benchmark status` 也能知道证据缺口；当 benchmark evidence 已 ready 且 `/benchmark trends` 状态为 `insufficient_history` 或 `regression` 时，round 必须输出 `benchmark_trends` gate、对应 `benchmark_trends:` gap 和失败 gate 的直接修复动作，单样本历史不足时直接修复动作应优先指向 `deepcli round --json --run-benchmark --fail-on-command`，避免补样本后还要手动回到 round 复核；当所有 gates 通过且 round ready 时，`nextActions` 必须在 `deepcli preflight --json`、`deepcli gate --json` 后，根据默认 competitor baseline 文件是否存在选择 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`、`deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json` 或 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`；没有 goal 时 `goalStatus` 为 null 且不生成 goal gate，存在未 ready 的 goal 时必须输出 `goalStatus` 摘要、`goal_readiness` gate 和 `deepcli goal gate --json` 下一步动作。显式传入 `--run-benchmark`/`--run-suite`、`--preset`、`--presets`、`--fail-on-command` 或 `--fail-fast` 时，应先执行 benchmark suite 并写入本地 artifact，再输出同一份 round 报告；`--json` 输出稳定 `deepcli.round.v1` schema，包含 scorecard 摘要、benchmarkStatus 摘要、可选 benchmarkRun、可选 goalStatus、gates、gaps、nextActions 和 report；scorecard 摘要中的 categories 必须包含分类级 `nextActions`，供 TUI、外部 UI 和脚本在单份 round 报告中完成分类级修复引导；`--output` 必须限制在 workspace 内；`--fail-on-command` 在执行的 benchmark 命令失败或超时时保留报告并返回非零，`--fail-on-gaps`/`--strict` 在本轮未 ready 时保留报告并返回非零，供持续产品循环、CI 或发布脚本要求“产品评分 + benchmark 证据 + benchmark 趋势 + goal readiness”同时 ready。
- `/selftest [--json] [--output path] [--fail-on-issues]`：面向 deepcli 产品自身的本地验收入口，聚合命令注册、项目配置、默认 provider 凭据、可恢复 session、日志、测试发现和支持入口状态；不得创建 session 或调用 provider；`--json` 输出稳定 `deepcli.selftest.v1` schema，包含 ready/status、commands、config、provider、sessions、logs、tests、issues、nextActions、checklist 和 report；JSON 顶层 `nextActions` 必须使用可直接执行的 `deepcli ...`、`cargo ...` 或 `git ...` 命令，`checklist[]` 必须从这些可执行动作派生 `step`、`label` 和 `command`，诊断说明留在 `report`，不得输出需要解析的 slash-command prose；`--output` 必须限制在 workspace 内；`--fail-on-issues` 在命令面缺失、项目配置缺失、默认 provider API key 缺失或无可发现测试时保留报告并返回非零，供安装脚本、CI 和迁移后验收使用。
- `/preflight [--json] [--output path] [--dry-run] [--quick] [--fail-fast]` 与顶层 `deepcli preflight` / `deepcli release-check`：面向提交、推送、发布前的高频一键本地检查入口，不创建 session、不调用 provider；full mode 应按顺序聚合格式检查、Git whitespace diff 检查、clippy、`/selftest --json --fail-on-issues`、`/doctor --quick --json`、`/privacy --json --fail-on-findings` 和 `/gate --json`，默认 keep-going 以展示所有失败，存在 required check 失败时保留报告并返回非零；`--quick` 跳过较慢的 clippy/gate，并将 privacy 检查切换为 `/privacy --json --fail-on-findings --no-history`，只作为本地快速迭代路径，不能替代 full mode 的完整历史隐私扫描；`--dry-run` 只输出将运行的检查清单且不执行命令，顶层 `nextActions` 必须使用可直接执行的 `deepcli preflight ... --json` 命令，不得输出 slash-command prose；`--fail-fast` 在首个 required failure 后停止；`--json` 输出稳定 `deepcli.preflight.v1` schema，包含 mode、dryRun、counts、diagnostics、checklist、checks、nextActions 和 report；`checklist[]` 应从实际检查队列派生，每项包含 step、name、label、command、status 和 required，供 TUI、外部 UI 或脚本无需解析 `checks[]` 或 report 文本即可渲染发布检查清单；`diagnostics` 应汇总 totalDurationMs、measuredChecks、slowestCheck、largestOutputCheck 和 failedRequiredChecks；`--output` 必须限制在 workspace 内。
- `/completion [bash|zsh|fish|json|install|status] [--force] [--json] [--output path]`：本地生成、安装或检查 shell 补全脚本，或输出机器可读命令目录，覆盖顶层命令、provider 快捷入口、常用环境/诊断参数和通用选项；不得创建 session 或调用 provider；无参数输出安装示例；`json` 输出稳定 `deepcli.completion.v1` schema，包含 program、version、shells、providers、install 和 commands；`install [bash|zsh|fish]` 默认 dry-run，只展示将写入的用户 HOME 补全路径、字节数和 reload 提示，只有显式 `--force` 才写入 allowlisted shell completion 文件；`install --json` 输出稳定 `deepcli.completion.install.v1` schema，包含 shell、targetPath、status、dryRun、force、bytes、parentCreated、nextActions 和 report；`status [bash|zsh|fish] --json` 应比较已安装文件与当前生成脚本，输出稳定 `deepcli.completion.status.v1` schema，包含 shell、targetPath、status(missing/stale/up_to_date)、installed、upToDate、expectedBytes、installedBytes、nextActions 和 report；`install --json` 与 `status --json` 的顶层 `nextActions` 必须是可直接复制执行的 `deepcli ...` 命令，不得输出 `install with ...`、`refresh with ...` 或 `restart your shell ...` 这类说明文本，shell reload 说明应留在 report；`--output` 必须限制在 workspace 内，供安装脚本、外部 UI、文档生成和 shell integration 测试使用。
- `/version [--json] [--output path]` 与 `/about [--json] [--output path]`：输出比 `deepcli --version` 更完整的本地产品和支持元数据，包括 package version、workspace、项目配置存在性、默认 provider、默认模型、provider 数量、provider turn 超时、注册 slash 命令数和 next actions；该入口用于 issue、支持包、安装验收和新用户自检，不创建 session、不调用 provider；`--json` 输出稳定 `deepcli.version.v1` schema，顶层 `nextActions` 必须是可直接复制到 shell 的 `deepcli ...` 命令，不得输出 `/quickstart` 或 `run \`/...\`` 这类 slash-command prose；`checklist[]` 必须从这些可执行动作派生，每项包含 step、label 和 command，供安装验收和支持 UI 直接渲染；`--output` 必须限制在 workspace 内，`/about` 是 `/version` 的别名。
- `/init`：初始化当前项目的 `.deepcli/` 本地状态、忽略规则和配置骨架，并输出后续凭据、环境和测试建议；默认只做低风险本地 scaffold，支持 `--quick`/`--no-env` 跳过环境探测。
- `/status [--json] [--output path]`：展示当前或最近有记录活动的 session 状态、token 消耗、provider turn、请求体大小、上下文压缩和下一步诊断入口；无 active session 时应回退到最近真实会话，而不是只显示 `session: <none>`；`--json` 输出稳定 `deepcli.status.v1` schema，`session.nextActions` 必须使用可直接执行的 `deepcli next|session diagnose|usage|trace ...` 命令，不得输出 `run \`/...\`` 这类 slash-command prose；顶层 `checklist[]` 和 `session.checklist[]` 必须从 `session.nextActions` 派生 step、label 和 command，供观测面板直接渲染；`--output` 可把当前文本或 JSON 状态报告写入 workspace 内 artifact，且不能允许路径逃逸。
- `/usage [--json] [--output path] [session_id|--current]`：展示会话用量汇总和诊断摘要，支持指定 session；提示 provider 慢响应、请求体过大、上下文压缩、provider 探测失败、工具失败和测试失败等信号；一次性命令创建的空 session 不应遮蔽最近有活动或审计记录的会话；`--json` 输出稳定 `deepcli.usage.v1` schema，包含 provider turn、token、cache、request size、context compaction、diagnostics 和 next actions，`session.nextActions` 必须使用可直接执行的 `deepcli trace ...` 和 `deepcli session diagnose ...` 命令，不得输出 slash-command prose；顶层 `checklist[]` 和 `session.checklist[]` 必须从 `session.nextActions` 派生 step、label 和 command，供用量/慢响应面板直接渲染；`--output` 可把当前文本或 JSON 用量诊断写入 workspace 内 artifact，且不能允许路径逃逸。
- `/diagnose [--quick|--full-env] [--probe-provider] [--provider <name>] [--limit n] [--json] [--output path] [--bundle dir] [session_id|--current]` 与 `/support [bundle-dir] [diagnose options]`：全局一键诊断入口，默认快速检查 workspace 配置、凭据、provider readiness、测试发现和最近可诊断 session；没有历史 session 时必须输出 workspace-only 诊断和明确 next actions，而不是报错；`--full-env` 才执行可能较慢的 Docker/Colima 环境检查，`--probe-provider` 才执行在线 provider 探测；`/diagnose docker|compiler` 应作为 `/env check docker|compiler` 的只读环境诊断直达入口，而不是把 `docker`/`compiler` 当作 session id；`--json` 输出稳定 schema，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，不得输出 `first-run guide: \`/quickstart\``、`/diagnose ...` slash prose 或 `<name>` 占位动作；`checklist[]` 必须从可执行 next actions 派生，每项包含 step、label 和 command，供支持面板直接渲染；`--output` 可把当前文本或 JSON 诊断报告写入 workspace 内 artifact，且不能允许路径逃逸；`--bundle dir` 应在 workspace 内生成脱敏支持包，至少包含 issue 模板、version/about 元数据、diagnose、quickstart、status、usage、trace、logs、session list 和 manifest，便于用户反馈慢响应、凭据配置、工具失败、版本配置和环境问题；support bundle `manifest.json` 的顶层 `nextActions` 也必须只包含可直接执行的 `deepcli ...` 命令，并提供从这些动作派生的 `checklist[]`，人工说明应放入 `notes`，不得输出 `attach this support bundle ...`、`start from issue.md ...` 或带 `<dir>` 的说明文本；`/support` 应作为 `/diagnose --bundle` 的直达快捷入口，默认写入 `.deepcli/support/latest`，让用户不必记忆长选项。
- `/next [--json] [--output path] [session_id|--current]`：作为 `/session next` 的快捷入口，聚合当前或最近可行动会话中的待审批、开放 by-the-way 问题、失败/拒绝工具、失败测试、未完成计划和 paused/failed/waiting_user 状态，并给出可直接复制的恢复、诊断和查看命令；无当前 session 或当前为空时应优先回退到最近有 next action 信号的会话，再回退到最近有记录活动的会话；`--json` 输出稳定 `deepcli.next.v1` schema，包含 session metadata、signals、checklist、nextActions、quickLinks 和原始 report，其中 `nextActions` 与 `quickLinks` 必须是可直接执行的 `deepcli ...` 命令，`checklist[]` 必须从 `nextActions` 派生 step、label 和 command，说明性原因放在 `signals` 和 `report`；`--output` 可把当前文本或 JSON next action 报告写入 workspace 内 artifact，且不能允许路径逃逸。
- `/doctor` 与 `/health`：诊断项目配置、权限、凭据、provider readiness、测试发现和环境状态；报告应直接包含 deepcli package version、注册命令数、默认 provider/model 相关配置和 provider turn timeout，避免高频健康检查还要额外运行 `/version` 才能反馈问题；支持 `--quick`/`--no-env` 跳过可能较慢的 Docker/Colima 环境检查，支持 `--fix` 自动补齐低风险本地项目结构，支持 `--probe-provider` 显式执行在线 provider 探测；`/doctor shell` 与 `/health shell` 应作为本地安装健康检查，默认 quick，检查 `deepcli` 是否在 PATH、是否解析到当前 workspace 的 launcher/binary、旧命令残留和 bash/zsh/fish completion 是否 missing/stale/up_to_date；`/health` 应作为 `/doctor --quick` 的直达入口，`/doctor docker|compiler` 和 `/health docker|compiler` 应作为 `/env check docker|compiler` 的只读环境诊断直达入口，避免把目标名当作非法 option；并根据环境检查、shell 安装状态和已发现测试给出可直接执行的下一步命令；next actions 应始终包含 `deepcli quickstart`，让首次诊断用户能回到完整启动/配置/编程/验收路线；支持 `--json` 输出稳定 `deepcli.doctor.v1` schema，包含 version、provider、readiness、sessions、tests、shell、environment、next actions 和 checklist；顶层 `nextActions` 必须是可直接复制到 shell 的命令，普通诊断动作使用 `deepcli ...`，`doctor shell --json` 可输出 PATH 修复所需的 `mkdir`、`chmod`、`ln`、`rm` 等 shell 命令；`checklist[]` 必须从这些可执行动作派生，每项包含 step、label 和 command，配置、凭据、环境和 shell 说明留在结构化字段或 `report` 中；支持 `--output path` 将当前文本或 JSON 输出写入 workspace 内 artifact。
- `project.gitIdentity`：项目配置可声明预期 Git 提交身份，包括 `userName` 和 `userEmail`；`/doctor` 与 `/selftest` 应在 Git 仓库内对比有效 `git config user.name` / `user.email`，发现缺失或不匹配时输出 issue 和可复制的 `git config` 修复命令；非 Git 目录只报告 `no_git`，不得读取或展示全局 Git 身份；该检查不得创建 session 或调用 provider。
- `/trace [--limit n] [--json] [--output path] [session_id|--current]`：展示会话审计时间线，用于定位 provider 响应、provider 探测、工具调用、测试、审批和模型切换耗时问题；一次性命令创建的空 session 不应遮蔽最近有审计记录的会话；`--json` 输出稳定 `deepcli.trace.v1` schema，包含 sessionSource、limit、total/shown events、脱敏后的审计事件 payload 和原始文本 trace，`--output` 可把当前文本或 JSON trace 写入 workspace 内 artifact，且不能允许路径逃逸。
- `/logs [--list|--file name] [--limit n] [--json] [--output path]`：本地只读查看 `.deepcli/logs`，默认 tail 最近修改的日志文件，`--list` 只列文件，`--file` 选择指定日志；输出必须脱敏，不创建 session、不调用 provider；`--json` 输出稳定 `deepcli.logs.v1` schema，包含日志目录、文件列表、选中文件、tail 行、截断状态、checklist、next actions 和原始报告，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，`checklist[]` 必须从这些动作派生 step、label 和 command，`--output` 必须限制在 workspace 内。
- `/privacy [scan] [--json] [--output path] [--fail-on-findings] [--limit n] [--no-history]`：本地只读扫描 git 提交元数据、remote URL、当前 tracked 敏感路径、历史敏感路径、绝对本机用户目录路径、项目配置禁用词和疑似密钥/私钥标记；输出样本必须脱敏，不创建 session、不调用 provider；`privacy.allowedEmails` / `privacy.allowedEmailDomains` 可声明公开或允许的邮箱，命中后应计入 suppressed findings 而不是 high/medium/low findings；`privacy.allowedCommitEmails` / `privacy.allowedCommitDomains` 只作用于提交元数据；`privacy.blockedTerms` 可声明旧产品名、公司邮箱、作者姓名或内部代号等项目特定禁用词，命中提交标题或历史文件内容时按 medium finding 处理且样例只显示 `<blocked-term>`；`privacy.allowedTerms` 可把确认保留的迁移说明或测试夹具折叠到 suppressed findings；`privacy.allowedUserPaths` 可声明脱敏后的本机用户路径，命中后同样进入 suppressed findings；`--json` 输出稳定 `deepcli.privacy.scan.v1` schema，包含 findings、suppressedFindings 和 counts；`--output` 必须限制在 workspace 内，`--fail-on-findings` 在未 suppressed 的 high/medium 风险存在时保留报告并返回非零，便于开源前检查和 CI gate。
- `/permissions`：查看或调整当前目录权限；`/permissions [show] [--json] [--output path]` 应输出默认权限模式、sandbox 能力、风险策略、哪些操作需要审批以及 next actions，JSON 使用稳定 `deepcli.permissions.show.v1` schema，`--output` 必须限制在 workspace 内；`set-mode` 继续用于修改默认权限模式。
- `/credentials`、`/login`、`/logout`、`/auth`、`/apikey` 与 `/key`：查看 provider 凭据状态、生成本地模板、从环境变量导入 API key，通过隐藏输入框/标准输入安全写入 API key，或移除本地文件中的 API key；`/credentials status [provider] [--json] [--output path]` 应输出每个 provider 的文件、环境变量、apiKey 配置状态、模型、endpoint、解析错误和 next actions，JSON 使用稳定 `deepcli.credentials.status.v1` schema，`--output` 必须限制在 workspace 内；`/login [provider] [--stdin] [--force]`、`/auth`、`/apikey` 和 `/key` 都应作为 `/credentials set` 的直达入口，`/logout [provider]` 应作为 `/credentials remove` 的直达入口，省略 provider 时使用当前 provider override 或默认 provider；`/credentials remove` 应只清除本地 credentials 文件中的 `apiKey`，保留 provider/model/endpoint 等元数据，并在对应环境变量仍存在时提醒用户环境变量仍会生效；任何凭据写入或移除入口都必须本地执行，不创建 session、不先调用 provider；任何输出、日志、trace 和会话记录都不得暴露明文凭据。
- `/config`：查看有效配置、来源、校验结果，并安全读取或修改单个配置项；`/config show|sources|validate|get <path>` 应支持 `--json` 输出稳定 `deepcli.config.inspect.v1` schema，并支持 `--output path` 将当前文本或 JSON 输出写入 workspace 内 artifact；结构化输出不得包含明文凭据，只能展示配置、来源、存在性和 configured/missing 状态。
- `/timeout [show|set <seconds>|reset] [--json] [--output path]`：作为 `agent.providerTurnTimeoutSeconds` 的高频本地入口，用于慢响应排查和临时调整 provider turn 超时；`/timeout` 与 `/timeout show` 展示当前有效超时、配置路径和排查 next actions，`/timeout <seconds>` 与 `/timeout set <seconds>` 写入项目 `.deepcli/config.json`，`/timeout reset` 恢复默认配置；所有形态都不得创建 session 或调用 provider，运行中会话执行写入时应重新加载 runtime config 并记录脱敏 audit；`--json` 输出稳定 `deepcli.timeout.v1` schema，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，不得输出 `<seconds>` 这类占位动作，`--output` 必须限制在 workspace 内。
- `/model`、`/provider`、`/use`、`/switch`、`/models` 与 `/providers`：查看、列出并切换当前会话使用的 provider/model；`/model [show|list] [--json] [--output path]` 应输出默认 provider、当前会话 provider/model、provider 类型、模型、凭据/环境变量配置状态、capabilities、next actions 和原始报告，JSON 使用稳定 `deepcli.model.inspect.v1` schema，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，不得输出 `<provider>` 或 `<model>` 这类占位动作，`--output` 必须限制在 workspace 内；`/models` 和 `/providers` 应作为 `/model list` 的只读直达入口，不创建空 session、不调用 provider；`/model set <provider> [model]`、`/model <provider> [model]`、`/provider <provider> [model]`、`/use <provider> [model]` 与 `/switch <provider> [model]` 均应作为本地模型切换入口，更新当前会话与项目配置；作为 one-shot 命令时必须在构造 AgentRuntime 前完成，不创建空 session、不调用 provider。
- `/goal [objective...]`：为当前会话设置长期目标契约，保存目标、需求来源、停止条件和验收命令；`/goal status` 应基于需求来源文件、守护计划步骤和 acceptance command 测试证据输出稳定 readiness 报告；`/goal gate` 应在仍有 blocker 时返回非零，避免 Agent 或脚本在目标未被证据证明前停止；无 active session 或当前 session 没有 goal 时，`show/status/gate` 应回退到最近一个带 goal 的会话并标注 session 来源，创建和清理 goal 不应回退写入历史会话。
- `/plan`：无参数查看当前执行计划；带粗糙需求文本时进入需求澄清模式，输出带推荐选项的澄清问题、假设、功能要求、验收标准和需求草稿，并可写入 docs。
- `/diff [--staged] [--path path] [--stat|--name-only] [--limit n]`：查看待应用或已应用修改；普通 `/diff` 优先显示当前 Git diff，在 Git diff 不可用或为空时回退展示会话记录的 diff 历史；`/diff --staged` 保持 Git staged diff 语义；`--path` 可重复传入工作区相对路径前缀，只显示匹配路径的 Git diff 或 session diff；大 diff 场景下可先用 `--stat` 或 `--name-only` 渐进查看，再用 `--limit` 限制完整 diff 输出。
- `/review [--path path]`：触发 auto-reviewer 或人工 review；优先审查当前 Git diff，在 Git diff 不可用或为空时回退审查会话记录的 diff 历史，避免非 Git 目录和空 one-shot 会话丢失审查上下文；`--path` 可重复传入工作区相对路径前缀，与 `/verify --path` 保持一致，便于从 scoped 验收继续 scoped review；同类发现应去重计数并只展示少量脱敏示例，避免大 diff 中重复噪声淹没关键风险；风险扫描应解析 diff 文件路径，只对新增危险命令报警，敏感信息扫描应区分真实密钥值和源码中的字段名/状态文本，并降低测试/文档路径、测试上下文和检测器字面量造成的误报。
- `/accept [verify options]` 与 `/gate [verify options]`：面向“我该如何验收”的高频入口，分别作为 `/verify --run-tests` 和 `/verify --run-tests --fail-on-blockers` 的易记别名；如果用户显式提供 `--test-command <command>` 或 `-- <command>`，应使用该命令作为测试证据而不是重复注入默认测试发现；无当前会话且无显式 session id 时，应以本次 workspace-only 新测试证据为准，不回退到历史 session 造成陈旧失败污染；二者复用 `/verify` 的 path/env/json/output/schema/blocker 语义，不创建 session、不调用 provider；`/gate` 在存在 blocker 时必须保留报告输出并返回非零退出码，适合 CI、安装验收和最终交付脚本。
- `/verify [--run-tests|--test-command <command>] [--env-check [docker|compiler]] [--path path] [--limit n] [--json] [--output path] [--fail-on-blockers] [session_id|--current]`：生成聚合验收报告，汇总 Git status、Git diff 或 session diff fallback、auto-reviewer 风险、最近测试记录、可选环境 readiness、失败工具、待审批、开放旁路问题和未完成计划；`--path` 可重复传入工作区相对路径前缀，只审查匹配路径的 Git diff 或 session diff，便于在大 diff 中做模块级验收；`--run-tests` 使用自动发现的测试命令，`--test-command` 使用指定命令，二者都必须走 `run_tests` 工具和权限策略，并把本次测试结果纳入报告；`--env-check docker|compiler` 使用只读 `check_environment` 工具把 Docker/编译器环境证据纳入报告，环境未 ready 或检查失败时必须成为 blocker，并给出 `/setup ... --smoke` 或 `/env plan ... --smoke --json` next action；`printf ok`、`echo ok`、`true` 等只证明 shell 可执行的 smoke 命令必须标为弱测试证据，不能解除验收 blocker；已有强测试通过但早于当前 diff 或 scoped diff 最新变更时，也必须标为过期证据并要求重新跑测试；无 session 时，若本次报告显式运行并通过了强测试，可降级为 workspace-only verification 提示而不是硬 blocker；auto-reviewer 的 high finding 必须进入 blockers，medium finding 应作为 review warnings 展示并给出 next action，避免警告噪声把真正的验收阻断淹没；该命令只给出证据和 next actions，不应在测试缺失、环境未 ready 或存在 blocker 时暗示可以验收；脚本和 CI 可使用 `--json` 读取结构化 `status`、`hasBlockers`、`blockers`、`environment`、`nextActions` 和 `checklist`，JSON 顶层 `nextActions` 必须是可直接执行的 `deepcli ...`、`cargo ...` 或 `git ...` 命令，不得输出说明性 prose、反引号 slash 命令或 `<...>` 占位动作，`checklist[]` 必须从这些动作派生 `step`、`label` 和 `command`，让 TUI、外部 UI 或脚本无需解析 report 即可渲染验收动作队列；使用 `--output` 把所选格式写入 workspace 内 artifact，且不能允许路径逃逸，并使用 `--fail-on-blockers` 在仍有 blockers 时返回非零退出码；`--json --output ... --fail-on-blockers` 在返回非零退出码时仍必须向 stdout 和输出文件写入有效 JSON。
- `/handoff [--path path] [--limit n] [--env-check [docker|compiler]] [--format text|markdown|json|pr] [--output path] [--fail-on-blockers] [session_id|--current]`：生成交付摘要，面向用户汇报、PR 描述或脚本自动化，汇总 workspace、session、Git 状态、diff 统计、review 风险、最近测试、可选环境 readiness 和 blockers；`--env-check docker|compiler` 使用只读 `check_environment` 工具把 Docker/编译器环境证据纳入交付报告和 PR 描述，环境未 ready 或检查失败时必须成为 blocker，并给出 `/setup ... --smoke` 或 `/env plan ... --smoke --json` next action；无测试、无强测试证据、强测试证据早于当前 diff 或 scoped diff 最新变更、环境未 ready、无 diff、无 session 或 high-risk review findings 时必须明确列为 blocker，不能生成“已完成”式结论。默认 text 输出保持适合终端阅读；`--markdown` 输出应适合直接粘贴到消息或评论；`--pr`/`--format pr` 输出应使用 Summary、Changes、Test Plan、Environment、Risks and Blockers、Checklist 结构，适合直接粘贴到 PR 描述；`--json` 输出应包含稳定 schema、status、hasBlockers、blockers、environment、nextActions 和 checklist，便于脚本消费，JSON 顶层 `nextActions` 必须是可直接执行的 `deepcli ...`、`cargo ...` 或 `git ...` 命令，不得输出说明性 prose、反引号 slash 命令或 `<...>` 占位动作，`checklist[]` 必须从这些动作派生 `step`、`label` 和 `command`，让交付 UI 不必自行命名 handoff 修复按钮；默认不写文件，显式 `--output` 时才把所选格式写入 workspace 内文件，且不能允许路径逃逸；`--fail-on-blockers` 在 blockers 非空时保留报告输出并返回非零退出码，便于 CI 或交付脚本 gate。
- `/test`：发现并运行测试命令；`/test [discover] [--json] [--output path]` 应输出发现到的测试命令、来源、Docker 需求、可用性、next actions 和原始报告，`/test run [--json] [--output path] [-- <command>]` 应输出执行命令、exit code、stdout/stderr、passed 状态、next actions 和原始报告，JSON 使用稳定 `deepcli.test.inspect.v1` schema；JSON 顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，发现到具体测试命令时应给出 shell-quoted 的 `deepcli test run --json -- <command>`，不得输出 `run \`/test ...\`` 这类 slash-command prose；`--output` 必须限制在 workspace 内；测试运行必须继续走工具权限策略，并在 active session 可用时记录测试证据。
- `/env`：检查、规划、安装和验证 Docker/编译器环境；所有形态都应作为本地 one-shot 命令运行，不创建空会话、不先调用 provider；`/env check [docker|compiler] [--json] [--output path]` 和 `/env plan [docker|compiler] [--smoke] [--json] [--output path]` 是只读预检，`/env setup [docker|compiler] [--smoke] [--json] [--output path]` 与 `/env test [docker|compiler] [--json] [--output path]` 继续走权限策略。`/check [docker|compiler]` 和 `deepcli check ...` 应作为 `/env check ...` 的直达入口；`/docker`、`/compiler`、`deepcli docker` 和 `deepcli compiler` 应作为 target-first `/env check <target>` 的只读入口，`/docker setup --smoke`、`/compiler test --json` 等 action 形式应映射到 `/env <action> <target>`；`deepcli test docker|compiler` 应映射到 `/env test docker|compiler`，但 `deepcli test run|discover` 仍保留项目测试语义。文本输出应同时展示 recommended 和 next actions，让用户看到可直接执行的 `/setup <target> --smoke` 以及预览用 `/env plan <target> --smoke --json`；JSON 使用稳定 `deepcli.env.inspect.v1` schema，包含 target、ready/status、checks、recommended action、would-run steps/actions、stdout/stderr 摘要、next actions 和原始报告；JSON 顶层 `nextActions` 必须使用可直接复制到 shell 的 `deepcli ...` 命令，`commands` 和 report 可保留 slash 命令用于 TUI 语境；`--output` 必须限制在 workspace 内，用于 TUI 环境面板、CI artifact、安装验收和下一步测试 gate。
- `/env`、`/setup`、`/install`：检测、计划、安装、配置和验证本地任务环境，例如 Docker/Colima 和 compiler-dev 镜像；`/setup [docker|compiler]` 应作为 `/env setup` 的直达入口，`/install [docker|compiler]` 应作为 `/env install` 的直达入口，二者继续复用同一权限策略、环境工具和 JSON/output 行为；`/env plan` 必须在执行安装或拉镜像前展示 would-run 步骤、风险和后续命令。
- `/git status|diff|branch|message [--json] [--output path]`：查看只读 Git 状态、diff、分支和提交信息建议；`--json` 必须输出稳定 `deepcli.git.inspect.v1`，包含 `kind`、实际执行命令、exit code、stdout/stderr、原始 raw、report 和可直接执行的 `deepcli ...` next actions；`--output` 必须把当前选择的只读输出写入 workspace-contained 文件并拒绝路径穿越；`diff` 支持 `--staged|--cached`；只读子命令遇到未知 option 或多余参数必须报错，不得静默忽略。`/git create-branch <name> [--dry-run] [--json] [--output path]` 和 `/git commit <message> [--dry-run] [--json] [--output path]` 应支持安全预览，dry-run 使用稳定 `deepcli.git.action.v1` schema、返回 planned command、nextActions 和 report，且不得创建分支或提交；真实写操作仍拒绝未支持的多余参数，并继续走权限策略。Agent 运行期间允许不带 `--output` 的只读 `/git status|diff|branch|message` 立即执行；`/git ... --output`、`/git create-branch <name>` 和 `/git commit <message>` 继续作为写入或受控写操作，需要等待当前任务结束或先 `/stop`。
- `/web`：通过受权限控制的网络工具执行 Web 搜索；支持 `/web search <query>`、`/web <query>` 和 `/search <query>`，查询内容先经过敏感信息拦截，结果应在摘要为空时回退展示相关主题。
- `/prompt`：管理自定义 prompt 和内置 prompt；自定义 prompt 可覆盖同名内置 prompt，并可删除恢复内置默认；`/prompt list|get <name>|render <name> ... [--json] [--output path]` 应输出 prompt 来源、路径、正文长度、渲染上下文、渲染结果、next actions 和原始报告，JSON 使用稳定 `deepcli.prompt.inspect.v1` schema，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，并在有具体 prompt 时优先给出具体名称，不得输出 `<path>`、`<name>` 这类占位动作，`--output` 必须限制在 workspace 内；Agent 工具层应能列出、读取和渲染可复用 prompt，渲染时支持 workspace、cwd、branch、diff、file、file_content 和自定义变量。
- `/skill`：发现、生成、注册、调用 Skill；`/skill list|run <name> [--json] [--output path]` 应输出 Skill 元数据、路径、触发条件、最大深度、指令正文、next actions 和原始报告，JSON 使用稳定 `deepcli.skill.inspect.v1` schema，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，并在有具体 skill 时优先给出具体名称，不得输出 `<name>`、`<description>` 这类占位动作，`--output` 必须限制在 workspace 内；Agent 工具层应能先列出已注册 Skill，再读取并执行匹配 Skill 的指令。
- `/agent`：查看和创建子 Agent 任务描述符；`/agent list|show <id> [--json] [--output path]` 应输出子任务 id/shortId、父 session、任务描述、深度、写入范围、状态、创建/更新时间、持久化路径、next actions 和原始报告，JSON 使用稳定 `deepcli.agent.inspect.v1` schema，顶层 `nextActions` 必须是可直接执行的 `deepcli ...` 命令，并在有具体任务时优先给出短 id，不得输出 `<task>` 或 `<id>` 这类占位动作，`show` 应支持唯一短 id 前缀，`--output` 必须限制在 workspace 内；`spawn` 继续作为写操作走工具权限与最大深度限制。
- `/approval`：查看、批准、拒绝和清理待审批请求；`/approval list [--json] [--output path] [session_id|--current] [--all]` 未显式指定 session 时，应避免 one-shot 空 session 遮蔽最近待处理审批，并支持稳定 `deepcli.approval.list.v1` schema、workspace 内 artifact 输出和顶层可执行 `nextActions`；有 pending 请求时应优先给出具体 approve/deny 命令，空队列时仍应给出复查完整队列和帮助命令，且不得输出 `<id>` 这类占位动作；`approve/deny <id> [--json] [--output path]` 应能通过审批 id 在历史会话中定位唯一请求，并输出稳定 `deepcli.approval.action.v1` schema、处理后的 approval、session、nextActions 和 report；`clear [--json] [--output path]` 应输出同一 action schema 和 cleared count；`deepcli --help` 应同时展示 list、approve、deny 和 clear，让用户不必进入二级帮助就能发现审批处理闭环。
- `/btw`：旁路记录、查看、回答和清理 by-the-way 小问题；`/btw list [--json] [--output path] [session_id|--current] [--all]` 未显式指定 session 时，应避免 one-shot 空 session 遮蔽最近开放问题，并支持稳定 `deepcli.btw.list.v1` schema、workspace 内 artifact 输出和顶层可执行 `nextActions`；空队列时仍应给出复查完整队列和帮助命令，且不得输出需要用户替换文本的 answer 占位动作；`answer <id> [--json] [--output path] <answer>` 应能通过问题 id 在历史会话中定位唯一问题，并输出稳定 `deepcli.btw.action.v1` schema、处理后的 question、session、nextActions 和 report；`clear [--json] [--output path]` 应输出同一 action schema 和 cleared count；`deepcli --help` 应同时展示 ask、list、answer 和 clear，让用户不必进入二级帮助就能发现旁路问题处理闭环。
- `/fork [session_id|--current] [--dry-run|--no-open] [--verify] [--app name] [--json] [--output path]`：复制已持久化会话上下文到新 session id；默认打开新 macOS Terminal 执行 `deepcli resume <new_id>`；终端 app 优先级必须是 `--app`/`--terminal-app`、`DEEPCLI_TERMINAL_APP`、`TERM_PROGRAM` 自动推断、Terminal，`TERM_PROGRAM=iTerm.app` 应自动选择 iTerm2，`TERM_PROGRAM=Apple_Terminal` 应选择 Terminal，未知 `TERM_PROGRAM` 应回退 Terminal；JSON 必须输出 `terminal.app` 和 `terminal.autoResumeSupported`，Terminal 与 iTerm2 支持自动执行 resume，其他 app 必须通过错误说明和 `workspaceResumeCommand` 引导手动恢复，不得伪装成已支持自动 resume；TUI 内无参数或 `--current` 使用 active session，shell 中无 id 时应选择当前 workspace 最近的可恢复对话上下文，并跳过空会话、tool-only 或诊断型 session；shell 中误用 `--current` 时应给出可执行的替代命令提示，并把 `deepcli fork --dry-run --json` 作为首个 JSON next action；`--dry-run` 只预览源会话、复制模式、计划标题、终端 app、是否默认打开终端和 next actions，不创建 session、不复制文件、不打开 Terminal；dry-run 的 fork next actions 必须保留显式、配置或自动推断的 `--app`，避免预览和真实执行不一致；`--no-open` 用于真实创建 fork 但跳过 Terminal；真实 fork 的 JSON 必须在 `terminal.workspaceResumeCommand` 中输出 shell-safe 的 `cd <workspace> && deepcli resume <new_id>`，并把同一条命令作为顶层 `nextActions[0]`，让手动恢复命令不依赖用户当前目录；`--verify` 用于真实 fork 后输出 resume 健康检查，确认 fork 是否 ready、workspace/provider/model 是否一致，以及消息、工具、测试、diff、backup 等持久化记录计数是否复制一致；JSON 应包含 `dryRun`、`contextCopy`、可选 `verification`、`hotForkSupported=false`、源会话状态和 next actions，明确不热复制 running Agent 任务；源会话正在运行时，真实 fork 和 dry-run JSON 顶层 `nextActions` 也必须只包含可直接执行的 `deepcli ...` 或 workspace resume shell 命令，运行中限制说明放在 `contextCopy.warning` 与 `report`；当 `--json` 下没有 active session 或没有可恢复源会话时，应保留 `deepcli.session.fork.v1` schema 输出 `status=error`、`source=null`、`fork=null`、`error.code`、`error.message` 和 next actions，并在写入 `--output` 后返回非零；一般 no-source next actions 应优先包含 `deepcli resume --dry-run --json` 和 `deepcli session list --all --limit 20 --json`，且不得包含 `<session_id>` 这类占位动作，让外部 UI 或脚本无需打开 TUI 也能继续发现候选。
- `/resume [session_id] [--dry-run|--preview] [--json] [--output path]`：恢复会话；TUI 选择器应按最近活动排序，并且无显式 id 时只在当前 workspace 的会话中选择候选，默认隐藏空 one-shot 会话、只包含工具/测试/审计记录的诊断型 session、只包含低信息输入和 deepcli 本地澄清回复的会话，以及短小已完成的单轮任务会话，支持直接输入字符按 title、id、provider、model 过滤，并在确认前预览所选会话的状态、活动量、summary 和最近消息；确认恢复后消息区应加载该会话完整已持久化用户/assistant 历史；启动入口 `deepcli resume` 在未指定 session id 时也应先打开同一套可过滤选择器，而不是自动恢复最近会话；手输 session id 时应支持唯一短前缀并在歧义时明确提示；`--dry-run --json` 应输出稳定 `deepcli.resume.preview.v1`，展示将恢复的 session metadata、activity、summary、最近消息、resume command 和 next actions，不创建 session、不进入 TUI、不调用 provider；无显式 id 时应使用同一套当前 workspace 可恢复候选去噪规则，显式 id 仍可预览指定 session，即使该 session 来自旧 workspace metadata 或属于默认隐藏的短会话；当 `--json` 下无显式 id 且没有可恢复候选时，应保留同一 schema 输出 `status=error`、`selected=null`、`error.code`、`error.message`、可执行 `nextActions` 和 `report`，并在写入 `--output` 后返回非零；`--output` 必须限制在 workspace 内。
- `/stop`：中断当前 TUI 中正在运行的 Agent 任务，将会话标记为 paused，并允许之后通过 `/resume` 继续。
- `/session`、`/history` 与 `/cleanup`：查看、导出和排查会话；首次真实用户任务应自动生成可读 session title，标题从用户任务归一化、截断并脱敏，用户仍可用 `/rename` 或 `/session rename` 覆盖；`/history` 应作为 `/session list` 的会话历史直达入口；`/session list [--all] [--limit n] [--json] [--output path]` 默认隐藏空 one-shot 会话并支持 `--all` 查看完整列表、`--limit n`/`-n n` 限制长列表输出，列表应同时展示可复制的短 id 与完整 id，`--json` 输出稳定 `deepcli.session.list.v1` schema，供 resume picker、外部历史页和脚本消费；`/session search <query> [--limit n] [--json] [--output path]` 可跨标题、summary、消息、工具、测试、diff 和 backup 搜索历史会话，文本和 JSON 命中结果都必须脱敏，`--json` 输出稳定 `deepcli.session.search.v1` schema，并提供可执行 `nextActions`：有命中时围绕首个命中给出 resume preview、history、next 和 diagnose，无命中时给出会话列表与 resume preview；`/session next [--json] [--output path] [session_id|--current]` 可聚合恢复建议和 next actions，JSON `nextActions` 与 `quickLinks` 必须使用可直接执行的 `deepcli ...` 命令，并从 `nextActions` 派生顶层 `checklist[]`；`/session diagnose [--limit n] [--json] [--output path] [session_id|--current]` 输出信号计数、最近失败工具、最近测试、未完成计划和快速诊断命令，`--json` 输出稳定 `deepcli.session.diagnose.v1` schema，其中 `recommendedNextActions` 与 `quickLinks` 同样使用 `deepcli ...` 命令，并从 `recommendedNextActions` 派生顶层 `checklist[]`，`--output` 可把当前文本或 JSON 会话诊断写入 workspace 内 artifact，且不能允许路径逃逸；`/session show|history|summary|tools|tests|diffs|backups` 都应支持 `--json` 输出稳定 `deepcli.session.inspect.v1` schema，并支持 `--output` 把当前文本或 JSON 查看结果写入 workspace 内 artifact，且不能允许路径逃逸；`/session tools --failed [--limit n]` 直达最近失败或被拒绝的工具调用并给出下一步诊断建议，未显式指定 session 时应回退到最近存在失败工具的会话；`/session rename <session_id|--current> <title>` 可直接重命名历史会话而不必先恢复它；`/cleanup [sessions] [--dry-run|--force] [--json] [--output path]` 应作为 `/session prune-empty` 的易记维护入口；`/session prune-empty [--dry-run|--force] [--json] [--output path]` 可预览并显式确认清理无 activity 且无标题的空会话，默认必须 dry-run 并跳过当前会话和有标题的空会话；`--json` 输出稳定 `deepcli.session.prune_empty.v1` schema，包含候选删除、跳过当前、跳过有标题空会话、实际删除数量和 next actions，顶层 `nextActions` 必须使用可直接执行的 `deepcli cleanup sessions ...`、`deepcli session list ...` 和 `deepcli history ...` 命令，供外部历史页、脚本和 TUI 清理确认使用；所有需要 session id 的查看、导出、审批、旁路和恢复操作都应支持唯一短前缀并在歧义时明确提示；`/session diffs` 可回看会话中记录的历史文件 diff；`/session backups` 可回看文件修改前的备份内容；`/session restore-backup <name|latest> [--path <target>] [--dry-run] [--json] [--output path]` 可预览或通过权限工具链恢复备份，新备份应记录原始目标路径以便省略 `--path`；`--dry-run --json` 应输出稳定 `deepcli.session.restore_backup.v1`，包含选定 session、backup、target、脱敏 diff 和 next actions，不写文件；真实恢复的 `--json` 也应保留同一 schema 并继续通过工具执行器写入；Agent 运行中仅允许 read-only `/session` inspection 和不带 `--output` 的 restore-backup dry-run 预览，`rename`、`export`、`prune-empty --force`、真实恢复和任何 `--output` artifact 写入必须等待任务结束或先 `/stop`；未显式指定 session 时，应避免 one-shot 空 session 遮蔽最近有消息、工具调用、测试、diff、backup、summary 或审计活动的会话，并支持 `--current` 强制查看当前 session。
- `/terminal [--dry-run|--no-open] [--app name] [--json] [--output path]`：打开同目录终端；终端 app 采用同一优先级，`DEEPCLI_TERMINAL_APP` 可显式设置默认 macOS 终端 app，`TERM_PROGRAM` 仅推断 Terminal/iTerm2 这类已支持终端，`--app`/`--terminal-app` 可单次覆盖，命令必须 shell-quote 为 `open -a <app> .`，并拒绝空 app 或控制字符；dry-run 不创建进程，JSON 输出稳定 `deepcli.terminal.v1`，包含 workspace、platform、supported、app、command、opened、workspaceCommand、nextActions 和 report；dry-run、失败和真实打开成功时的顶层 `nextActions` 都必须只包含可直接执行的 `cd <workspace>` 或保留显式、配置或自动推断 app 的 `deepcli ...` 命令，不得输出 `use the opened terminal ...` 这类说明文本；`--output` 只能写入 workspace 内路径。

### 5.4 Agent 编程能力

- 代码库扫描和摘要。
- 调用链、数据流、模块依赖分析。
- 修改计划生成。
- 文件读写。
- diff 展示。
- 测试执行。
- 测试失败分析和修复循环。
- 最终变更汇报。

### 5.5 工具系统

- 文件系统工具。
- shell 工具。
- Git 工具。
- 网络搜索工具。
- provider API 工具。
- 测试命令发现工具。
- 环境管理工具：检测本机依赖、安装缺失工具、启动本地运行时、拉取任务镜像并执行 smoke test。
- Skill 调用工具。
- 子 Agent 调度工具。

### 5.6 Sandbox 系统

- Agent 默认在 sandbox 中运行。
- sandbox 约束文件系统、shell、网络、Git、Docker 和依赖安装能力。
- sandbox 内允许的操作按配置执行；缺少权限时进入审批流。
- 审批优先交给 auto-reviewer；auto-reviewer 无法确定或风险较高时交给用户。
- 高风险操作即使在完全控制权限下也必须二次确认。
- sandbox 决策、审批记录和权限升级必须写入会话和日志。

### 5.7 Provider 系统

- DeepSeek 作为默认 provider。
- DeepSeek adapter 优先完整实现；Kimi adapter 先保留骨架和配置读取能力。
- 支持 Kimi 作为本地配置 provider。
- 抽象 provider 接口，预留 OpenAI、Anthropic、本地模型。
- 支持流式输出、tool calling、JSON 输出、reasoner、上下文缓存。
- 支持 token 统计和阈值提醒。
- 端到端验收阶段允许使用 DeepSeek API，并允许配置 DeepSeek V4 Pro 作为执行 Agent 的模型；实际 API model id 以 provider 配置为准。

### 5.8 配置系统

- 项目级配置目录：`.deepcli/`。
- 项目配置：`.deepcli/config.*`。
- 凭据文件：`.deepcli/credentials/`，必须被 Git 忽略。
- 用户规则：`.deepcli/AGENTS.md` 或项目根目录 `AGENTS.md`。
- Skill 配置：`.deepcli/skills/`。
- Agent 配置：`.deepcli/agents/`。
- Prompt 配置：`.deepcli/prompts/`。
- 会话数据：`.deepcli/sessions/`，默认不提交。
- 日志和 trace：`.deepcli/logs/`，默认不提交；`/logs` 提供本地只读、脱敏 tail/list/json/output 入口，便于用户在支持包生成前快速查看最近日志。

## 6. 用户角色和权限

### 6.1 用户角色

- CLI 用户：主要使用者，发起任务、审批操作、查看结果。
- Agent：根据用户目标执行分析、计划、修改、测试和汇报。
- Auto-reviewer：自动审核低风险操作，不能确定时升级给用户。
- 子 Agent：在最大深度限制内执行受控子任务。

### 6.2 权限模式

- 只读权限：允许读取当前授权目录内文件，不允许写入和执行危险命令。
- 写权限：允许在授权目录内修改文件，需遵循 diff 和审批策略。
- 完全控制权限：允许无需逐次审批执行大部分操作，但高风险操作仍需二次确认。
- Sandbox 模式：默认工作模式。Agent 先在 sandbox 授权范围内执行；若工具调用超出 sandbox 能力，进入 auto-reviewer 或用户审批。

### 6.3 高风险操作

以下操作必须二次确认：

- `rm -rf` 或等价递归删除。
- `git reset --hard`。
- 系统目录写入。
- 修改用户主目录敏感配置。
- 删除大量文件。
- 安装或升级系统级依赖。
- 推送到远程仓库或创建远程 PR。

## 7. 业务流程

### 7.1 首次进入目录

1. 用户在项目目录执行 `deepcli`。
2. CLI 检查 `.deepcli/config.*` 和全局配置。
3. CLI 检查目录授权状态。
4. 若未授权，向用户申请读取当前目录权限。
5. CLI 加载 ignore 规则和敏感文件规则。
6. Agent 扫描项目上下文。
7. Agent Runtime 初始化默认 sandbox。
8. 进入交互式会话。

### 7.2 执行编程任务

1. 用户输入任务。
2. Agent 分析上下文。
3. 对复杂任务先输出计划，说明调用链、数据流和影响范围。
4. 用户确认计划或调整目标。
5. Agent 按计划读取文件、修改文件、运行测试。
6. 工具调用先尝试在 sandbox 内执行。
7. 若 sandbox 缺少权限或操作命中风险策略，触发权限判断。
8. 低风险权限升级由 auto-reviewer 审批，高风险操作交给用户审批。
9. 测试失败时进入修复循环。
10. 完成后输出变更摘要、验证结果和风险。
11. 可选执行 Git commit。

### 7.3 长任务续跑

1. Agent 将任务状态、计划、工具调用和测试结果写入会话。
2. 用户中断或退出。
3. 用户后续执行恢复命令。
4. CLI 加载会话状态。
5. Agent 从上次计划节点继续执行。

## 8. 状态流转

### 8.1 会话状态

- `new`：新建会话。
- `context_loading`：加载配置、权限和项目上下文。
- `waiting_user`：等待用户输入。
- `planning`：生成或更新计划。
- `awaiting_approval`：等待审批。
- `executing`：执行工具或修改文件。
- `testing`：运行验证。
- `reviewing`：执行 review。
- `paused`：用户暂停。
- `failed`：任务失败但可恢复。
- `completed`：任务完成。

### 8.2 工具调用状态

- `requested`：Agent 请求工具调用。
- `policy_checking`：检查权限策略。
- `auto_approved`：auto-reviewer 自动审批。
- `user_approved`：用户审批通过。
- `denied`：审批拒绝。
- `running`：工具运行中。
- `succeeded`：运行成功。
- `failed`：运行失败。

## 9. 数据需求

### 9.1 配置数据

- provider 配置。
- model 配置。
- API 凭据文件路径。
- 权限模式。
- token 阈值。
- sandbox 规则。
- auto-reviewer 策略。
- 子 Agent 最大深度。
- 网络和代理配置。

### 9.2 会话数据

- 用户消息。
- 模型回复。
- 工具调用请求与结果。
- 审批记录。
- 文件 diff。
- 计划状态。
- 测试命令和结果。
- token 消耗。
- 错误和恢复点。

### 9.3 Prompt 和 Skill 数据

- 内置 prompt。
- 用户自定义 prompt。
- Skill 元数据。
- Skill 指令文档。
- Skill 调用记录。

### 9.4 隐私和忽略数据

- `.env*`、证书、私钥、token、credentials、SSH key 等默认不得上传。
- 大文件、构建产物、依赖目录默认忽略。
- 支持项目级 `.deepignore`。

## 10. 接口/API 需求

### 10.1 Provider API

- Chat completion。
- Streaming。
- Tool calling。
- JSON/schema 输出。
- Reasoner 模型。
- 上下文缓存。
- token 用量返回。
- 限流、重试和退避。

### 10.2 内部接口

- `ProviderClient`：统一模型调用。
- `ToolRegistry`：注册和发现工具。
- `PermissionEngine`：权限判断和审批。
- `SessionStore`：会话持久化。
- `WorkspaceContext`：仓库上下文加载。
- `PatchWriter`：文件写入和 diff 管理。
- `CommandRouter`：`/` 指令分发。
- `SkillRegistry`：Skill 管理。
- `AgentRuntime`：Agent 循环和状态机。

## 11. 页面/交互需求

本项目主要是 CLI/TUI 交互，不做 Web 页面。

交互要求：

- 默认中文输出，跟随用户语言调整。
- 清晰展示计划、工具调用、审批请求、diff、测试结果。
- message box 支持多行输入、光标移动、局部编辑和常用 IDE 组合键。
- `/status` 可展示 token、上下文、任务状态。
- Agent 运行时可处理 by-the-way 小问题，并回到主任务。
- 用户中断时保存现场并提示恢复方式。

## 12. 技术设计建议

- 使用模块化架构，明确 CLI、TUI、Agent runtime、provider、tool、permission、session、skill、git 的边界。
- Provider 使用适配器模式，DeepSeek 为默认适配器，Kimi 和其他 provider 复用同一接口。
- 工具调用必须先经过权限引擎，不允许 Agent 直接绕过权限执行。
- 文件修改使用直接写入，但写入前后生成 diff，并以追加式历史记录写入会话，不能因同一文件多次修改而覆盖旧 diff。
- shell 执行需要命令分类和风险等级识别。
- ignore 和隐私规则必须在上下文收集前生效。
- 会话存储使用结构化 JSONL 或 SQLite；MVP 可先用本地文件，后续再升级。
- 所有敏感凭据只落在本地 `.deepcli/credentials/`，不进入日志、会话和 Git。

## 13. 测试计划

### 13.1 单元测试

- provider 适配器。
- 配置加载。
- ignore 规则。
- 权限策略。
- 命令风险识别。
- 会话保存和恢复。
- `/` 指令解析。
- Skill 注册与调用。

### 13.2 集成测试

- 一次性任务执行。
- REPL 多轮会话。
- 文件修改和 diff。
- shell 审批。
- 测试失败修复循环。
- Git 状态和 commit message 生成。
- 会话中断恢复。

### 13.3 验收测试

核心验收任务：

- 通过调用本项目产出的 `deepcli` 产品，在本地 Git 仓库中启动 Agent。
- Agent 根据 `work/myWork/compiler` 项目中的需求文档和 `online-doc` 要求，独立 coding 生成完整 Rust 编译器实现代码，从 lv1 到 lv9+。
- Agent 在验收过程中可以连接 web 获取必要公开资料，但必须遵守隐私过滤和 sandbox/approval 策略。
- Agent 根据需求文档中的环境配置，独立配置 Docker 环境、拉取 image，并运行本地自动化测试。
- 验收执行期间允许调用本项目配置的 DeepSeek API，并允许使用 DeepSeek V4 Pro 作为 Agent 执行模型；实际 API model id 以 provider 配置为准。
- 如果验收过程中发现 `deepcli` 产品能力不足、流程中断、权限策略错误、工具调用失败或 Agent 无法继续，应回到本项目修复和完善 CLI，再重新执行验收测试。
- 测试只要求本地仓库验证，不需要提交远程。
- 任务过程中必须体现计划、数据获取、实现、测试、review、修复和最终汇报闭环。

## 14. 验收标准

- 能在 macOS 当前目录启动并进入交互式会话。
- 能读取 `.deepcli/config.*` 和 provider 凭据。
- 能完成 DeepSeek 流式模型调用。
- 能申请目录读取权限并遵守 ignore 规则。
- Agent 默认在 sandbox 中工作，sandbox 缺少权限时能通过 auto-reviewer 或用户 approval 升级。
- 能生成复杂任务计划并执行完整编程循环。
- 能修改文件、展示 diff、运行测试、修复失败。
- 能保存和恢复会话。
- 能通过 `/status` 展示 token、上下文和任务状态。
- 能管理自定义 prompt 和内置 prompt。
- 能生成和调用 Skill。
- 能按最大深度 spawn 子 Agent。
- 能执行受控 Git 工作流。
- 能对高风险命令进行二次确认。
- 能以本项目 CLI 为执行入口，完成 `work/myWork/compiler` 从需求文档到 Docker 自动化测试通过的端到端验收流程。
- 不泄露本地 API Key、隐私文件和被 ignore 的内容。

## 15. 风险和待确认事项

- MVP 范围很大，需要按内部里程碑拆分，否则实现周期和验证成本较高。
- DeepSeek 的 tool calling、JSON 输出、上下文缓存和 token 统计细节需要以实际 API 能力为准。
- macOS TUI 对 message box、组合键和新终端打开方式有实现差异，需要技术验证。
- 自动审批和完全控制权限存在安全风险，需要默认保守。
- 长任务续跑需要稳定的状态机和会话格式，否则容易出现恢复不一致。
- 子 Agent 并发会带来文件写入冲突，需要锁或任务所有权机制。
- Rust 编译器 lv1 到 lv9+ 是高强度验收任务，需要后续拆成阶段计划。
