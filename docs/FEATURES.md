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
- `deepcli resume [session_id] --dry-run --json`：预览将恢复的会话上下文，不进入 TUI、不调用 provider。
- `deepcli deepseek ...`：使用 DeepSeek provider 预设。
- `deepcli kimi ...`：使用 Kimi provider 预设。
- `deepcli recipes [topic]`：查看任务型工作流命令清单。
- `deepcli goal [objective...]`：为当前会话写入长期目标契约和验收停止条件。
- `deepcli plan <rough requirement>`：围绕不成熟需求生成澄清问题、推荐选项和需求草稿。
- `deepcli fork [session_id|--current] [--dry-run|--no-open] [--verify]`：预览或复制已持久化会话上下文，并可打开新终端恢复到副本，在同一历史上下文上独立继续交互；`--verify --json` 会输出 resume 健康检查。
- `deepcli terminal [--dry-run|--no-open] [--json]`：打开当前 workspace 的新终端，或输出可脚本验收的 `deepcli.terminal.v1` 预览；JSON 包含可直接复制的 `workspaceCommand` 和首个 `cd <workspace>` nextAction。
- `deepcli version|about|health|doctor [--json]`：输出本地版本、配置、凭据、环境和支持诊断信息；JSON 顶层 `nextActions` 是可直接复制到 shell 的命令，说明性上下文留在 `report`、`environment` 或 `shell` 字段。
- `deepcli scorecard [--json]`：查看产品能力覆盖、SOTA 差距和 benchmark 证据。
- `deepcli round [--json] [--fail-on-gaps]`：聚合 scorecard、benchmark status 和最近 goal readiness，输出本轮产品迭代状态、去重后的门禁和下一步动作。
- `deepcli benchmark presets|run-suite|run|record|status|gate|summary|trends|baseline-template|compare|list|show|clean [--json]`：发现推荐 workload、一键执行推荐基准套件、执行单项 preset、记录、评估证据质量、门禁、汇总、趋势分析、baseline 模板、baseline 对比、列出、查看和清理本地 benchmark 证据 artifact。

启动 wrapper 会自动补充当前工作目录、配置路径和 yes 授权默认值，同时保留显式参数。

## TUI 与交互体验

TUI 面向实际编码任务，而不是简单聊天框：

- message box 支持编辑、粘贴、多行输入和历史输入。
- slash command palette 支持过滤、选择和补全。
- 会话消息会从持久化记录恢复。
- Agent 运行中仍可执行本地安全命令，例如 `/status`、`/usage`、`/trace`、`/logs`、`/privacy`、`/fork`、`/recipes`、`/scorecard`、`/round`、`/benchmark status|summary|trends|compare|list|show|presets`、`/selftest`、`/preflight --dry-run`、`/completion`、read-only `/session`、`/session restore-backup --dry-run --json`、`/approval`、`/terminal`、`/stop` 和 `/quit`。
- 会执行本地 shell、修改 session metadata、导出/写入 artifact、删除会话或恢复文件的 `/round --run-benchmark`、`/benchmark run*|record|baseline-template|clean`、`/session rename`、`/session export`、`/session prune-empty --force`、`/session ... --output`、`/session restore-backup` 真实恢复和完整 `/preflight` 需要等当前 Agent 任务结束或先 `/stop`。
- 任务观察面板的 quick actions 会按动作类型展示 `Enter run`、`Enter edit` 或 `Enter run/edit`，避免可编辑命令被误认为会直接执行。
- 工具调用默认以可扫描的任务观察面板呈现，并支持查看工具详情；Tools 视图在折叠列表状态直接展示 `/session tools --limit 20 --current` 和 `/session tools --failed --limit 20 --current` 可编辑动作，鼠标点击会预填 message box，展开详情时仍保留 `Ctrl-O`/`Ctrl-F` 快捷入口。

## 会话管理

会话是 deepcli 的核心状态单元：

- `deepcli resume` 打开当前 workspace 的会话选择器，并默认跳过只包含工具、测试或审计记录的诊断型 session、只包含低信息输入和本地澄清回复的会话，以及短小已完成的单轮任务会话。
- `deepcli resume <session_id>` 恢复指定会话。
- `deepcli resume <session_id> --dry-run --json` 输出稳定 `deepcli.resume.preview.v1`，展示将恢复的 session、activity、summary、最近消息和 next actions，不创建新会话、不进入 TUI、不调用 provider；无 id 时同样使用当前 workspace 内去噪后的可恢复候选；没有可恢复候选时同一 schema 输出 `status=error`、`selected=null`、`error.code` 和可执行 `nextActions` 后返回非零。
- `deepcli sessions --all --limit 20` 查看历史。
- `deepcli history` 是历史列表快捷入口。
- 顶层命令支持常规帮助旗标，例如 `deepcli fork --help`、`deepcli sessions -h` 和 `deepcli deepseek fork --help` 都会转到对应 `/help` 主题。
- `/rename` 可重命名当前或指定会话。
- `/goal` 可把当前会话绑定到长期目标，默认目标是完整实现项目文档需求，并要求验收命令和测试全部通过后才可结束。
- `/fork` 会复制当前或指定会话目录中的持久化上下文，给副本生成新 id/title，并默认打开新 macOS Terminal 执行 `deepcli resume <new_id>`；TUI 内的 `/fork` 或 `/fork --current` 使用 active session，shell 中的 `deepcli fork` 无 id 时会选择当前 workspace 最近的可恢复对话上下文，并跳过空会话和诊断型 session；`--dry-run --json` 只预览源会话、复制模式、计划标题和下一步动作，不创建 session；源会话选择失败时仍输出 `deepcli.session.fork.v1`、`status=error`、`error.code` 和 `nextActions`，且 no-source JSON 动作优先给出 `deepcli resume --dry-run --json` 和 `deepcli session list --all --limit 20 --json`，方便脚本和外部 UI 不打开 TUI 也能继续发现候选；`--no-open` 会真实创建 fork 但跳过 Terminal；真实 fork 的 JSON 会在 `terminal.workspaceResumeCommand` 中给出 `cd <workspace> && deepcli resume <new_id>`，并把同一条命令放在顶层 `nextActions[0]`，方便用户从任意 shell 目录手动恢复副本；`--verify --json` 会在真实 fork 后输出 `verification`，检查 workspace、provider/model、fork state、resume command，以及消息、工具、测试、diff、backup 计数是否复制一致；JSON 中的 `contextCopy` 会说明源会话状态、复制模式和是否处于运行中任务；Agent 运行中也可立即 fork 已落盘上下文，让新终端基于同一历史副本独立继续交互，但当前运行中的 Agent 任务不会被热分叉。
- 源会话处于运行中时，fork JSON 的顶层 `nextActions` 仍只给可执行命令，例如 `deepcli stop` 和 `deepcli fork --current`；不热复制内存任务的说明保留在 `contextCopy.warning` 和 `report`。
- `/session search` 可按标题、摘要、消息、工具调用、测试、diff 等搜索历史；JSON 会给出围绕首个命中的 resume preview、history、next/diagnose 动作，无命中时给出会话列表和 resume preview 动作。
- `/next --json` 和 `/session next --json` 的 `nextActions`/`quickLinks` 使用可直接执行的 `deepcli ...` 命令；`/session diagnose --json` 的 `recommendedNextActions`/`quickLinks` 也使用同一命令格式，解释性原因保留在 `signals` 和 `report`。
- `/session restore-backup latest --dry-run --json` 会输出稳定 `deepcli.session.restore_backup.v1` 预览，包含选中的 backup、目标文件、脱敏 diff 和下一步恢复命令；真实恢复也支持 `--json`/`--output`，但仍通过受控工具执行器写文件并记录新的 backup/diff。Agent 运行中可直接执行不带 `--output` 的 dry-run 预览；`/session rename`、`/session export`、`/session prune-empty --force`、`/session ... --output`、真实恢复和预览 artifact 写入都需要等待任务结束或先 `/stop`。
- `/cleanup sessions` 可预览或删除空的一次性会话；JSON 顶层 `nextActions` 使用可直接执行的 `deepcli cleanup sessions --force`、`deepcli session list ...` 和 `deepcli history ...` 命令。

## Provider、模型与凭据

deepcli 当前面向 DeepSeek-compatible providers，并内置 DeepSeek/Kimi 相关入口：

- `deepcli model show|list`
- `deepcli use <provider> [model]`
- `deepcli switch <provider> [model]`
- `deepcli provider [provider] [model]`
- `deepcli providers --json`

Status、Usage、模型、超时、日志、Prompt、Skill 和 Agent 查看类 JSON 的结构化 `nextActions` 都是可直接执行的 `deepcli ...` 命令；`status.session.nextActions` 会根据会话信号给出 `deepcli next/session diagnose` 或 `deepcli usage/trace`，`usage.session.nextActions` 会给出 `deepcli trace` 和 `deepcli session diagnose`，有具体条目时会优先输出具体 prompt 名称、skill 名称或 agent 短 id，说明性上下文留在 `report` 或条目字段。

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

`quickstart --json` 和 `selftest --json` 的顶层 `nextActions` 使用可直接执行的 `deepcli ...`、`cargo ...` 或 `git ...` 命令；quickstart 的首次引导说明保留在 `steps` 和 `report`，selftest 的诊断说明保留在 `report`，避免外部 UI、安装脚本或验收脚本解析 slash-command prose。

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
环境 JSON 顶层 `nextActions` 使用可直接复制到 shell 的 `deepcli ...` 命令，例如 `deepcli setup docker --smoke` 和 `deepcli env plan docker --smoke --json`；`commands` 与报告正文仍保留 slash 形式，便于 TUI 内继续使用。
`health/doctor --json` 复用同一可执行动作契约：缺凭据时给出 `deepcli credentials set/import-env/template ...`，环境未就绪时给出 `deepcli setup ... --smoke` 或 `deepcli env test compiler`，`doctor shell --json` 的 PATH 和 completion 建议直接给出可复制命令。

## 测试、验收与交付

deepcli 不只负责生成代码，也负责形成交付证据：

- `deepcli recipes sota --json`
- `deepcli recipes release --json`
- `deepcli goal "完整实现当前项目文档中的全部需求" --json`
- `deepcli goal status --json`
- `deepcli goal gate --json`
- `deepcli plan "做一个需求澄清功能" --write-doc docs/ai/PLANNED_REQUIREMENTS.md`
- `deepcli resume --dry-run --json`
- `deepcli fork --current --dry-run --json`
- `deepcli fork --current --no-open --json`
- `deepcli fork --current --no-open --verify --json`
- `deepcli terminal --dry-run --json`
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
- `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`
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

`goal` 输出稳定 `deepcli.goal.v1` JSON，并把目标、需求来源、停止条件和验收命令保存到当前 session 的 `goal.json` 与守护 `plan.json`。后续 Provider 上下文会收到 active goal contract，约束 Agent 不能在目标、验收要求和测试全部满足前声称结束。`goal status` 输出稳定 `deepcli.goal.status.v1`，检查需求来源文件、goal 守护计划步骤和每条 acceptance command 的最新测试证据；`goal gate` 复用同一报告，并在仍有 blocker 时返回非零，适合用作“是否允许停止”的本地门禁。`goal show/status/gate` 在无 active session 或当前 session 没有 goal 时，会回退到最近一个带 goal 的会话，并在 JSON 中标注 `sessionSource`；创建和清理 goal 不回退，避免 one-shot 命令误写历史会话。`plan` 输出稳定 `deepcli.plan.requirements_draft.v1`，面向粗糙需求生成澄清问题、多个候选选项、首推选项、假设、功能要求、验收标准和下一步动作；在有当前 session 时，澄清问题也会进入旁路问题队列，用户可继续回答。`resume --dry-run --json` 输出稳定 `deepcli.resume.preview.v1`，从持久化 session 文件读取 metadata、activity、summary 和最近消息，供外部 UI 或脚本在进入 TUI 前确认将恢复的上下文；无 id 时只在当前 workspace 中选择候选，并跳过只包含工具、测试或审计记录的诊断型 session、只包含低信息输入和本地澄清回复的会话，以及短小已完成的单轮任务会话；当无可恢复候选时保留同一 schema 输出 `status=error`、`selected=null`、`error` 和 `nextActions` 后非零退出。`fork` 输出稳定 `deepcli.session.fork.v1`，复制已持久化会话上下文但不复制 metadata id，适合把同一上下文分支给新的终端继续探索；无 id 且没有 active session 时，fork 使用同一类可恢复对话候选，避免把空诊断 session 当作默认源；dry-run 报告使用同一 schema、`status=dry_run` 和 `dryRun=true`，且 `fork=null`，用于确认源会话和计划而不创建历史记录；预期源会话选择失败时会使用同一 schema 输出 `status=error`、`source=null`、`fork=null`、`error` 和 `nextActions` 后非零退出，其中 no-source 动作优先给出 `deepcli resume --dry-run --json` 和 `deepcli session list --all --limit 20 --json`；真实 fork 可加 `--verify` 输出 `verification` resume 健康检查，确认副本是否 ready、workspace/provider/model 是否一致、持久化记录计数是否复制一致；`contextCopy` 与 `nextActions` 会明确暴露源会话状态、复制模式、运行中任务限制和恢复命令；Agent 运行中允许 fork 当前已落盘上下文，但不热复制后台 Agent 任务。运行中 `/session` 仅允许 read-only inspection 和不带 `--output` 的 restore-backup dry-run 预览，`rename`、`export`、`prune-empty --force`、真实恢复和任何 `--output` artifact 写入都会提示等待任务结束或先 `/stop`。`terminal` 输出稳定 `deepcli.terminal.v1` JSON，允许外部 UI 或验收脚本在不打开 Terminal 的情况下确认 workspace、命令、平台支持状态和可复制的 `workspaceCommand`。

`preflight` / `release-check` 是提交/推送前的一键本地检查入口，串联 `cargo fmt --check`、`git diff --check`、`cargo clippy --all-targets -- -D warnings`、`selftest`、`doctor --quick`、`privacy --fail-on-findings` 和 `gate --json`，并输出稳定 JSON 报告；`--dry-run` 只预览检查清单，且顶层 `nextActions` 给出可直接执行的 `deepcli preflight ... --json` 命令；`--quick` 跳过较慢的 clippy/gate，并把 privacy 检查切换为 `privacy --no-history`，用于快速本地迭代。提交、推送或发布前仍应运行 full preflight，因为 full mode 保留完整历史隐私扫描；文本和 JSON 报告包含 `diagnostics` 摘要，展示总耗时、最慢检查、最大输出检查和失败 required check，避免用户在长报告中手动查找瓶颈。

`recipes` / `playbook` 是任务型工作流目录，按 start、code、debug、release、support、environment、shell、sota 等主题输出可复制命令和稳定 `deepcli.recipes.v1` JSON，适合 TUI、外部 UI 或团队脚本引导用户选择下一步；`recipes.nextActions` 保持为可直接执行的 `deepcli ...` 命令，说明性上下文留在 `recipes[].notes` 和 report 中；`recipes sota` 可通过 `product-loop`、`benchmark` 或 `round` alias 进入，用于把产品缺口检查、benchmark evidence、baseline 模板、baseline compare 和 gate 串成一条本地闭环，并会把当前 `round` 的失败 gate 修复动作放在顶层 `nextActions` 前面，避免产品循环入口先推荐已知无效的只读报告；默认 competitor baseline 缺失且当前 artifact 可完整捕获时，顶层 `nextActions` 会先提示 `baseline-template --from-current` 生成 compare-ready 本地基线，再保留生成 `.deepcli/baselines/competitor.json` 的手工模板动作，文件存在后再提示执行 baseline compare；该命令本地只读，不创建 session、不调用 Provider。

`scorecard` 是产品能力评分和 SOTA 差距入口，按命令发现、Agent 工作流、会话续跑、验收交付、安全隐私、Provider/模型、支持诊断和 benchmark 证据给出 0-100 分、tier、gaps、next actions 和稳定 `deepcli.scorecard.v1` JSON；`--fail-below` 可作为本地产品门禁，命令不创建 session、不调用 Provider。`scorecard.nextActions` 在存在 gaps 时会作为本轮修复队列，只聚焦当前 gaps 的直接修复、SOTA 产品循环和验收动作；各分类的 `nextActions` 仍会先展示本分类 gap 的修复动作，再展示通用探索命令；当只缺 benchmark evidence 时，首项直接指向可复制执行的 `deepcli round --json --run-benchmark --fail-on-command` 修复命令，不再把当前 `deepcli scorecard --json` 报告本身或只读 `deepcli round --json` 塞回 benchmark evidence 修复队列，并继续露出 `deepcli recipes sota --json` 作为完整产品循环导航；当没有 gaps 且状态为 ok 时，顶层 `nextActions` 会切换为 `round`、`preflight`、`gate`、SOTA recipe、benchmark trends/status，以及按默认 competitor baseline 是否存在选择的 `--from-current`、baseline template 或 baseline compare 等持续验收动作，不再重复输出各强分类的 discovery 命令。`round` 默认输出稳定 `deepcli.round.v1`，把 scorecard、benchmark status 和最近 goal readiness 聚合成本轮产品迭代报告，包含 ready 状态、门禁、gaps 和下一步命令；内嵌的 `scorecard.categories[]` 摘要会保留分类级 `nextActions`，让 TUI、外部 UI 或脚本只读取 round 报告也能按分类展示修复动作；`scorecard` gate 只检查分数阈值是否达标，benchmark evidence 和 goal readiness 由专属 gate 呈现，其它未满足项继续留在 gaps 列表中，避免同一问题在多个 gate 中重复失败；benchmark gate 会列出缺失、weak、stale、失败或超时的 required preset，让用户在同一份 round 报告里直接看到证据缺口；当 benchmark evidence 已 ready 但 trends 仍是 `insufficient_history` 或 `regression` 时，round 会额外输出 `benchmark_trends` gate、对应 gap 和直接修复动作，其中单样本历史不足会优先提示 `deepcli round --json --run-benchmark --fail-on-command`，让用户补样本后立即看到新的 round 结果。`round.nextActions` 按失败 gate 的修复路径排序；当 scorecard 已达标且剩余 gaps 全部属于 benchmark evidence 时，首个动作是 `deepcli round --json --run-benchmark --fail-on-command`，并省略重复的 `deepcli scorecard --json` 和自引用的 `deepcli round --json`，同时给出 `deepcli recipes sota --json`；当所有 gates 通过且 round ready 时，会在 `deepcli preflight --json`、`deepcli gate --json` 后继续按默认 competitor baseline 是否存在提示 `--from-current`、生成 baseline template 或执行 baseline compare。存在未 ready 的 goal 时会输出 `goalStatus` 摘要、`goal_readiness` gate 和 `deepcli goal gate --json` 下一步动作，没有 goal 时保持只读报告且不创建 session。显式加 `--run-benchmark` 或 `--run-suite` 时会先执行 benchmark suite，再在同一份 round JSON 中写入 `benchmarkRun` 和更新后的 `benchmarkStatus`；`--fail-on-command` 适合阻断 benchmark 命令失败，`--fail-on-gaps` 适合在持续产品循环或 CI 中要求本轮 evidence、产品分数和 goal readiness 都 ready。`benchmark` 保留无子命令和 scorecard flags 的兼容行为，并增加 `presets/run-suite/run/record/status/gate/summary/trends/baseline-template/compare/list/show/clean`：`presets` 列出 cargo-test、preflight-quick、selftest、scorecard 和 smoke 等推荐 workload，`run-suite` 默认连续执行 cargo-test、preflight-quick、selftest 和 scorecard，并输出稳定 `deepcli.benchmark.suite.v1` 汇总报告，也可重复传入 `--preset` 只跑指定子集，`run --preset <name>` 显式执行对应本地命令、采集 exit code、耗时和输出摘要并写入 `.deepcli/benchmarks/*.json`，`record` 只记录声明证据，`status` 输出稳定 `deepcli.benchmark.status.v1` 并把证据判定为 missing、weak、incomplete、failing、stale 或 ready，同时展示 required preset 覆盖细节和 `freshness`；任一 required preset 证据超过 1 天会标为 `aging`、保留 ready 语义并把 `deepcli round --json --run-benchmark --fail-on-command` 放到刷新动作前面；gap 修复提示使用可直接执行的 `deepcli benchmark ...` 命令，避免单项通过被误判为完整 suite 证据；`status` 与 `summary` JSON 都包含原始 `report` 文本，便于 TUI、外部 UI 和脚本直接展示同一份摘要；证据缺失时优先在 nextActions 中给出 `deepcli recipes sota --json` 且不展示 dry-run clean，已有 artifact 时才把 `deepcli benchmark clean --dry-run --json` 作为证据维护动作，`gate` 在 status 不是 ready 时返回非零，`summary` 聚合历史 artifact 的通过率、失败数、耗时范围和最新 artifact，`trends` 输出稳定 `deepcli.benchmark.trends.v1`，按 suite/case 展示最近状态回归、恢复和耗时变化；已有 artifact 但所有 case 都没有 previous 样本时，`trends` 返回 `insufficient_history` 并优先提示 `deepcli round --json --run-benchmark --fail-on-command`，让用户补样本后直接看到新的 round 结果，避免把单样本误报为趋势充分。`baseline-template` 输出可编辑的 `deepcli.benchmark.baseline.v1` JSON 并可写入 workspace 内 baseline 文件，`compare` 输出稳定 `deepcli.benchmark.compare.v1`，只读取本地 artifact 和 workspace 内 baseline JSON，按 suite/case 展示状态对比、缺失项和耗时差异；baseline 仍缺 `status` 或 `durationMs` 时，`compare` 会保持 `incomplete` 并在 nextActions 中提示先编辑对应 baseline 文件，`list/show` 用于本地验收和持续产品循环，`clean` 输出稳定 `deepcli.benchmark.cleanup.v1`，默认 dry-run 预览旧 artifact，只有显式 `--force` 才删除。

当 benchmark evidence 仍为 ready 但 freshness 为 aging/stale 时，`scorecard --json`、`round --json` 和 `recipes sota --json` 的顶层 `nextActions` 会先给出 `deepcli round --json --run-benchmark --fail-on-command`，避免用户在证据已经建议刷新时先进入 preflight、gate 或 baseline 对比。

`baseline-template` 写出的 `deepcli.benchmark.baseline.v1` 文件会带顶层 `status=needs_values`、`nextActions` 和 `report`，stdout 与 `--output` 文件保持一致；模板仍保留每个 required case 的待填写 `status` / `durationMs`，同时直接提示编辑目标 baseline 文件并运行 `deepcli benchmark compare --baseline <path> --json`。加 `--from-current` 时会从最新 required benchmark artifact 捕获每个 case 的 `status` 和 `durationMs`，证据完整时生成 `status=ready` 的 baseline，让本地当前版本、旧版本或手工跑完的对照版本可以不经手改直接进入 compare。

当 scorecard 自身已经 ok 但 benchmark trends 仍需要补历史或处理回归时，scorecard 顶层 `nextActions` 会优先给出 `deepcli round --json --run-benchmark --fail-on-command`，避免评分入口先把用户带到已知不能推进闭环的只读报告。

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
- `deepcli privacy` 可在开源或共享前扫描 git history、提交邮箱、本机绝对路径、敏感路径、配置的禁用词和疑似密钥，并支持 JSON artifact 与 `--fail-on-findings`。
- `privacy.allowedEmails` / `privacy.allowedEmailDomains` 可把确认公开或允许的邮箱从 `/privacy` findings 中折叠到 suppressed findings，`privacy.allowedCommitEmails` / `privacy.allowedCommitDomains` 可只允许提交元数据邮箱，降低开源前检查噪声。
- `privacy.blockedTerms` 可把旧产品名、公司邮箱、作者姓名或内部代号纳入本地隐私门禁；`privacy.allowedTerms` 可折叠确认保留的迁移说明或测试夹具，报告中只展示 `<blocked-term>` 占位符。
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
./scripts/deepcli fork --help
./scripts/deepcli sessions -h
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
./scripts/deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json
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
