# deepcli 当前已实现功能盘点

> 生成日期：2026-06-26
> 用途：给人工检查当前 deepcli 已落地功能、实现证据、验证状态和剩余风险。本文按当前工作树源码与轻量命令输出整理，不以历史聊天记忆为准。

## 1. 检查范围

本次盘点读取并交叉核对了以下内容：

- 产品文档：`README.md`、`docs/FEATURES.md`、`docs/ai/REQUIREMENTS.md`、`docs/ai/TECHNICAL_PLAN.md`、`docs/ai/CONTEXT.md`
- 核心源码：`src/cli.rs`、`src/commands.rs`、`src/runtime.rs`、`src/ui.rs`、`src/tools.rs`、`src/providers.rs`、`src/session.rs`、`src/config.rs`、`src/permissions.rs`、`src/workspace.rs`、`src/prompts.rs`、`src/skills.rs`、`src/agents.rs`、`src/privacy.rs`
- wrapper 与工程配置：`scripts/deepcli`、`Cargo.toml`、`.gitignore`
- 自动化测试：`tests/mvp_contract.rs`、`tests/wrapper_contract.rs`，以及源码模块内的单元测试
- 实际轻量命令输出：
  - `./scripts/deepcli --help`
  - `./scripts/deepcli version --json`
  - `./scripts/deepcli scorecard --json`
  - `./scripts/deepcli round --json`
  - `./scripts/deepcli benchmark status --json`
  - `./scripts/deepcli recipes sota --json`
  - `./scripts/deepcli model list --json`
  - `./scripts/deepcli prompt list --json`
  - `./scripts/deepcli test discover --json`

## 2. 总体结论

当前 deepcli 已经不是简单聊天 CLI，而是一个具备本地优先工作流、结构化 one-shot 命令、TUI、Provider、工具执行、权限审计、会话恢复、验收交付、benchmark 产品循环和诊断支持能力的 Rust CLI。

从实现证据看，当前能力大致分为三层：

- 已经有真实实现并有测试保护的能力：CLI/wrapper 命令入口、TUI 交互框架、会话持久化、工具注册和执行、权限判定、DeepSeek/Kimi provider、Prompt/Skill/Agent 查看与工具、Git/测试/环境/诊断/隐私/交付命令、benchmark/round/scorecard 产品循环。
- 已经有入口和本地闭环，但仍依赖实际运行环境或外部服务质量的能力：真实 provider 调用、Docker/编译器环境安装、Web 搜索、Git 写操作、完整 preflight、benchmark suite 刷新。
- 明确仍是后续增强方向的能力：成熟多 provider 生态、GitHub PR 远程协作、本地语义索引、自动升级、组织级权限、MCP/IDE/浏览器/桌面自动化等。

当前工作区的产品状态不是完全 ready：本地 benchmark artifact 存在，但 required presets 已超过 7 天，`benchmark status --json` 和 `round --json` 都把证据判定为 `stale`，首要建议是执行 `deepcli round --json --run-benchmark --fail-on-command` 刷新 benchmark 证据。

## 3. 架构与实现入口

| 模块 | 当前职责 | 主要证据 |
|---|---|---|
| `src/main.rs` / `src/cli.rs` | Rust CLI 入口、参数解析、provider/mode/top-level alias 归一化、one-shot 本地命令分流、交互入口选择 | `Cli`、`run_cli`、`normalize_cli_aliases`、`parse_one_shot_command` |
| `scripts/deepcli` | 启动 wrapper，自动构建二进制，设置 `-C "$PWD"`、默认 config、`--yes`，加载本地 provider key，映射常用帮助与顶层命令 | wrapper `usage`、`is_top_level_slash_command`、`help_topic_for_top_level_command` |
| `src/commands.rs` | slash command 解析与绝大多数本地命令实现，输出稳定 JSON schema、`nextActions`、`checklist`、`report` | `SlashCommand`、`CommandRouter::parse`、`CommandRouter::handle`、各 `handle_*` |
| `src/runtime.rs` | Agent runtime、会话监控数据、provider turn、工具调用循环、运行中状态观察 | `AgentRuntime`、`SessionMonitor`、`SessionObservationUsage` |
| `src/ui.rs` | TUI、message box、任务观察面板、Result/Changes/Usage/Health/Library/Deliver/Tools/Tests/Environment/Approvals/Trace tabs、resume picker、running-safe 命令 | `MonitorTab`、`MessageBox`、`run_tui`、相关 UI 测试 |
| `src/tools.rs` | 工具注册与执行，覆盖文件、shell、Git、测试、环境、Web、Terminal、Prompt、Skill、Subagent | `ToolRegistry::mvp`、`ToolExecutor` |
| `src/providers.rs` | Provider 抽象、DeepSeek OpenAI-compatible adapter、Kimi Anthropic-style adapter、流式解析、tool call 标准化、usage/token 估算、重试与代理配置 | `ProviderClient`、`DeepSeekClient`、`KimiClient` |
| `src/session.rs` | 会话 metadata、消息、工具调用、测试、diff、backup、goal、plan、审批、BTW 问题持久化与短 id 解析 | `SessionStore`、`Session`、相关 JSONL/metadata 方法 |
| `src/config.rs` | 默认配置、项目/全局配置加载、Provider runtime config、凭据引用、隐私和代理配置 | `AppConfig`、`ProviderConfig`、默认配置 |
| `src/permissions.rs` | 权限模式、sandbox 判定、风险分级、危险 shell/Git/Docker/安装命令处理 | `PermissionEngine`、`DecisionOutcome`、`RiskLevel` |
| `src/workspace.rs` | 工作区授权、文件上下文、ignore 规则、敏感文件默认排除 | `WorkspaceManager`、`DeepIgnore` |
| `src/prompts.rs` / `src/skills.rs` / `src/agents.rs` | Prompt、Skill、Subagent 描述符的本地库与基础 CRUD/读取/渲染能力 | `PromptStore`、`SkillStore`、`AgentStore` |
| `src/privacy.rs` | 文本脱敏与敏感值检测，用于日志、trace、diff、隐私扫描等输出 | `redact_sensitive_text`、`looks_sensitive` |

## 4. 已实现功能清单

### 4.1 CLI 与命令发现

已实现内容：

- 默认 `deepcli` 进入 TUI；`deepcli tui` 显式进入 TUI；`deepcli repl` 保留旧行式 REPL。
- `deepcli ask <prompt>` 与 `deepcli stream <prompt>` 用于 one-shot 任务；缺 prompt 会本地报错，不回退创建会话。
- DeepSeek/Kimi provider 前缀入口已归一化，例如 `deepcli deepseek ...`、`deepcli kimi ...`。
- 常用 slash command 都能通过顶层命令访问，例如 `deepcli doctor --quick`、`deepcli scorecard --json`、`deepcli benchmark status --json`、`deepcli session ...`。
- 常规帮助旗标会转到对应主题，例如 `deepcli fork --help`、`deepcli sessions -h`。
- 未知但像 CLI 命令的输入会在本地拒绝并给 nearest-command 建议，避免误发给 provider。
- `completion` 支持 bash/zsh/fish 脚本、JSON 命令目录、安装和状态检查。

实现证据：

- `src/commands.rs` 中 `SlashCommand` 已注册 70 个 slash 命令。
- `tests/mvp_contract.rs` 验证 MVP slash commands 和工具注册。
- `tests/wrapper_contract.rs` 验证 wrapper 映射、provider 前缀、help 转发、ask/stream 逃生路径。
- `./scripts/deepcli --help` 已实际展示当前命令面。

### 4.2 TUI 与交互体验

已实现内容：

- message box 支持多行、历史、光标编辑、粘贴、快捷键。
- TUI 任务观察面板包含 Overview、Result、Changes、Usage、Health、Library、Deliver、Tools、Tests、Environment、Approvals、Trace。
- Result 和消息区支持滚动；Changes tab 可展示 Git 工作区状态、文件列表和 patch 预览；Tools tab 默认折叠并支持展开详情。
- quick actions 可选择、运行或预填编辑；含风险或占位的动作倾向预填而不是直接执行。
- slash command palette 支持过滤、补全和鼠标选择。
- resume picker 支持筛选、预览、鼠标滚动与点击选择。
- Approvals tab 支持选择审批或 BTW 问题，批准/拒绝/回答需要显式键盘动作，鼠标点击不会直接执行安全敏感操作。
- Agent 运行中支持一批本地观察命令，例如 status、trace、approval、read-only session、read-only git、product loop 报告、terminal dry-run、fork persisted context。

实现证据：

- `src/ui.rs` 中 `MonitorTab` 覆盖完整 tab 集合。
- UI 单测覆盖 message box、粘贴、scroll、Changes/Tools/Approvals 交互、running-safe 命令、resume picker、dashboard render。
- `src/ui.rs` 中 running handler 明确阻止运行中 artifact 输出、completion force install、session 写操作、Git 写操作等。

### 4.3 Provider、模型与凭据

已实现内容：

- Provider trait 抽象支持 `chat`、`stream`、`count_tokens`、capability、metadata。
- DeepSeek adapter 支持 OpenAI-compatible chat/completions、streaming、tool call、JSON mode、reasoning 内容、usage、重试、HTTP/HTTPS proxy/no_proxy。
- Kimi adapter 已实现 Anthropic-style request/response/stream/tool-use 映射。
- 模型命令支持 `model show|list|set`、`use`、`switch`、`provider`、`providers`。
- 凭据命令支持 status、set/login/auth/apikey/key、remove/logout、template、import-env，输出脱敏。
- 当前本地命令 `model list --json` 显示已配置 2 个 provider：DeepSeek 与 Kimi。

实现证据：

- `src/providers.rs` 中 `ProviderClient`、`DeepSeekClient`、`KimiClient`。
- provider 单测覆盖 OpenAI-compatible tool call、DeepSeek SSE、Kimi streamed text/tool use、usage、retryable 状态、proxy no_proxy。
- credentials/model 单测覆盖凭据模板、环境导入、覆盖保护、脱敏、路径逃逸拒绝、模型读取和切换。

注意：

- OpenAI、Anthropic、本地模型等当前没有完整实现；除 DeepSeek/Kimi 外的 provider type 会返回未实现。
- 真实 provider 端到端质量需要有效 API key 和在线服务验证，本次只做了本地命令与实现层核对。

### 4.4 工具系统与 Agent 执行能力

已实现内容：

- 工具注册表当前包含 25 个 MVP 工具。
- 文件工具：读取、列文件、搜索、写文件、patch/full replacement，写入记录 diff 与 backup。
- shell 工具：受权限控制的命令执行，带超时。
- Git 工具：status、diff、branch、create branch、commit message、commit。
- 测试工具：测试命令发现与执行，测试结果写入 session。
- 环境工具：Docker/compiler readiness check、plan/setup/test。
- Web 工具：privacy-filtered search。
- Terminal 工具：打开同目录终端。
- Prompt/Skill/Subagent 工具：list/get/render、skill generate/run、spawn subagent。

实现证据：

- `ToolRegistry::mvp()` 中列出 `read_file`、`write_file`、`run_shell`、`git_*`、`discover_tests`、`run_tests`、`check_environment`、`setup_environment`、`web_search`、`open_terminal`、`prompt_*`、`skill_*`、`spawn_subagent`。
- `tests/mvp_contract.rs` 验证工具注册。
- `src/tools.rs` 单测覆盖 workspace 路径限制、patch、placeholder/大文件危险重写拒绝、test result 记录、审批队列、subagent 持久化、Prompt/Skill 工具、Web 搜索格式等。

### 4.5 权限、sandbox 与安全审计

已实现内容：

- 默认配置为 `sandbox`，工作区读允许，系统写和危险命令受限。
- 权限引擎区分 filesystem、shell、git、network、docker、terminal 等 surface。
- 低风险只读 shell 可允许；测试/构建类命令可自动审批；Docker、依赖安装、破坏性 shell、破坏性 Git 需要用户审批或二次确认。
- 工具调用生命周期会记录 requested、policy checking、approved/denied/running/succeeded/failed 等状态。
- 敏感输出会经 `privacy` 模块脱敏。

实现证据：

- `src/permissions.rs` 中 `PermissionEngine`、`RiskLevel`、`DecisionOutcome`。
- 权限单测覆盖 read-only shell、destructive shell、medium risk shell、Docker、package install、Docker environment setup。
- 工具单测覆盖无审批写入进入 pending approval、`assume_yes` 允许普通 workspace 写但不允许危险 shell。

### 4.6 会话、恢复、fork 与长期目标

已实现内容：

- 会话持久化包括 metadata、messages、tool calls、audit events、plan、goal、tests、diffs、backups、side questions、approval requests。
- 会话支持自动标题、短 id 前缀解析、rename、list/history/search/show/summary/tools/tests/diffs/backups。
- `resume` 支持 picker、显式 id、短前缀、dry-run JSON preview、candidates JSON。
- 默认 resume/fork 候选会跳过空 session、工具/诊断型 session、低信息澄清 session、短小已完成单轮任务、其它 workspace session。
- `fork` 支持 dry-run、no-open、verify、Terminal/iTerm2 app 推断、workspace resume command、running session warning。
- `terminal` 支持 dry-run/no-open/json/output/app 选择。
- `goal` 支持创建目标契约、status、gate，并在无 active session 时回退最近带 goal 的会话；`plan` 支持需求澄清草稿与旁路问题。

实现证据：

- `src/session.rs` 中会话数据结构与读写方法。
- `src/commands.rs` 中 `handle_resume`、`handle_fork`、`handle_goal`、`handle_plan_command`、`handle_terminal`、`handle_session`。
- 命令测试覆盖 goal gate、plan 草稿、fork clone/verify/running warning、resume preview/candidates/去噪、terminal dry-run/app 推断、session restore/prune/search/next/rename。

### 4.7 Product Loop：scorecard、round、benchmark、recipes、opportunities

已实现内容：

- `scorecard`：按命令发现、Agent workflow、会话连续性、验收交付、安全隐私、Provider/模型、支持诊断、benchmark evidence 等分类评分。
- `round`：聚合 scorecard、benchmark status、benchmark trends、可选 goal readiness，形成本轮产品迭代 gate/gap/opportunity/nextActions。
- `benchmark`：支持 presets、run-suite、run、record、status、gate、summary、trends、baseline-template、compare、baselines、list、show、clean。
- `recipes sota`：把产品循环串成可复制命令清单，并根据当前 round 状态调整 nextActions。
- `opportunities`：输出非阻塞产品机会、优先级/成本计数、推荐机会和 action checklist。
- 所有关键 JSON 都倾向输出 `nextActions` 和 `checklist[]`，供 TUI 或外部 UI 直接渲染按钮。

实现证据：

- `src/commands.rs` 中 `handle_scorecard`、`handle_round`、`handle_benchmark`、`handle_recipes`、`handle_opportunities`。
- 实际命令输出：
  - `scorecard --json` 输出 `deepcli.scorecard.v1`
  - `round --json` 输出 `deepcli.round.v1`
  - `benchmark status --json` 输出 `deepcli.benchmark.status.v1`
  - `recipes sota --json` 输出 `deepcli.recipes.v1`
- 测试覆盖 scorecard nextActions、round gates、benchmark suite/status/trends/baseline/cleanup、recipes SOTA 状态感知、opportunities 过滤。

当前状态：

- 当前本地 benchmark artifact 数量为 36。
- required presets 包括 `cargo-test`、`preflight-quick`、`selftest`、`scorecard`。
- 这些 required presets 当前都存在但超过 7 天，状态为 `stale`。
- 当前首要修复动作是 `deepcli round --json --run-benchmark --fail-on-command`。

### 4.8 测试、验收、交付与 Git

已实现内容：

- `test discover|run` 可发现和执行测试命令，当前工作区发现 `cargo test`。
- `preflight` 可串联 fmt、diff check、clippy、selftest、doctor、privacy、gate；支持 dry-run/quick/json。
- `verify` / `accept` / `gate` 聚合 diff、测试、环境、审批、旁路问题、失败工具等交付门禁。
- `handoff` 可生成文本、Markdown、PR-ready 输出和 JSON。
- `git status|diff|branch|message` 是只读 JSON inspect；`git create-branch|commit` 是受控写入口，支持 dry-run JSON 预览。
- Git identity 检查可对比项目配置的 user/email。

实现证据：

- `src/commands.rs` 中 `handle_test`、`handle_preflight`、`handle_verify`、`handle_handoff`、`handle_git`。
- 测试覆盖 test discover/run JSON、preflight diagnostics、verify/gate/handoff JSON、Git inspect/action JSON、Git identity 检查、diff/review/handoff 范围过滤。

### 4.9 配置、诊断、日志、隐私与支持包

已实现内容：

- `version/about` 输出版本、workspace、config、provider、命令数量和下一步动作。
- `doctor/health/diagnose/support` 覆盖本地健康、shell install、support bundle、provider readiness、session diagnostics。
- `logs` 支持本地脱敏查看。
- `trace` 支持审计事件查看，空 one-shot session 会回退到有实际 audit events 的会话。
- `privacy` 扫描 Git metadata、remote、tracked/historical sensitive paths、user-home path、blocked terms、token/private key 等，并支持 allowlist/suppression。
- `config` 支持 show/sources/validate/get/set，`timeout` 支持 show/set/reset，`permissions show` 输出权限配置。
- `.gitignore` 默认忽略 `.deepcli/credentials/`、sessions、logs、benchmarks、baselines、exports、support 和常见凭据/构建产物。

实现证据：

- `src/commands.rs` 中 `handle_version`、`handle_doctor`、`handle_diagnose`、`handle_logs`、`handle_privacy_scan`、`handle_config`、`handle_timeout`、`handle_permissions`。
- `src/privacy.rs` 与相关测试覆盖脱敏策略。
- 命令测试覆盖 doctor shell、support bundle、diagnose bundle、logs JSON、privacy history findings、blocked terms、output path traversal 拒绝。

### 4.10 Prompt、Skill 与 Subagent

已实现内容：

- Prompt：内置 prompt 包含 `code-review`、`fix-tests`、`implementation-plan`；支持项目 prompt 覆盖、list/get/render/delete，render 支持 file、diff、自定义变量等上下文。
- Skill：支持项目 skill list/run/generate；run 输出 metadata、instructions、nextActions。
- Agent：支持 subagent task descriptor 持久化、list/show、短 id 前缀。

实现证据：

- 实际 `prompt list --json` 输出 3 个内置 prompt。
- `src/prompts.rs`、`src/skills.rs`、`src/agents.rs`。
- 测试覆盖 prompt render、prompt JSON、skill list/run JSON、agent list/show JSON、subagent spawn 持久化。

## 5. 稳定 JSON schema 盘点

当前代码中已经出现并测试或命令可达的主要 schema 包括：

- `deepcli.version.v1`
- `deepcli.quickstart.v1`
- `deepcli.recipes.v1`
- `deepcli.scorecard.v1`
- `deepcli.scorecard.summary.v1`
- `deepcli.opportunities.v1`
- `deepcli.round.v1`
- `deepcli.benchmark.record.v1`
- `deepcli.benchmark.suite.v1`
- `deepcli.benchmark.status.v1`
- `deepcli.benchmark.summary.v1`
- `deepcli.benchmark.trends.v1`
- `deepcli.benchmark.baseline.v1`
- `deepcli.benchmark.compare.v1`
- `deepcli.benchmark.baselines.v1`
- `deepcli.benchmark.cleanup.v1`
- `deepcli.benchmark.presets.v1`
- `deepcli.selftest.v1`
- `deepcli.preflight.v1`
- `deepcli.completion.v1`
- `deepcli.completion.install.v1`
- `deepcli.completion.status.v1`
- `deepcli.resume.preview.v1`
- `deepcli.resume.candidates.v1`
- `deepcli.terminal.v1`
- `deepcli.status.v1`
- `deepcli.usage.v1`
- `deepcli.trace.v1`
- `deepcli.logs.v1`
- `deepcli.privacy.v1`
- `deepcli.doctor.v1`
- `deepcli.credentials.status.v1`
- `deepcli.model.inspect.v1`
- `deepcli.prompt.inspect.v1`
- `deepcli.skill.inspect.v1`
- `deepcli.agent.inspect.v1`
- `deepcli.test.inspect.v1`
- `deepcli.git.inspect.v1`
- `deepcli.git.action.v1`
- `deepcli.approval.list.v1`
- `deepcli.approval.action.v1`
- `deepcli.btw.list.v1`
- `deepcli.btw.action.v1`
- `deepcli.session.list.v1`
- `deepcli.session.search.v1`
- `deepcli.session.inspect.v1`
- `deepcli.session.diagnose.v1`
- `deepcli.session.prune_empty.v1`
- `deepcli.session.restore_backup.v1`
- `deepcli.goal.status.v1`

检查重点：

- 大多数 JSON schema 都同时提供 `report`、`nextActions`、`checklist[]`。
- 代码和测试反复约束 `nextActions` 应是可直接执行的 `deepcli ...` 命令，避免 `<placeholder>` 或 TUI slash prose 混入外部 UI。
- 带 `--output` 的命令普遍有 workspace-contained path 校验和路径逃逸拒绝测试。

## 6. 自动化测试覆盖概况

当前测试覆盖较密集，主要集中在以下方面：

- 命令注册与 wrapper 路由：MVP slash commands、provider alias、help topic、ask/stream、one-shot 本地命令不创建空 session。
- 命令 JSON 契约：scorecard、round、benchmark、selftest、preflight、completion、version、agent、test、git、privacy、prompt、skill、session、approval、btw、config、credentials、doctor、diagnose、env、verify、handoff。
- TUI 行为：message box、粘贴、滚动、tab、quick actions、running-safe、resume picker、Tools/Changes/Approvals 交互。
- Provider 解析：DeepSeek/OpenAI-compatible tool call 与 SSE，Kimi Anthropic-style tool use/stream。
- 权限与工具：workspace path、写入保护、大文件危险重写拒绝、审批队列、测试记录、subagent、Web 搜索 fallback。
- 会话持久化：metadata/messages/plan/goal/side questions/approvals/diffs/backups、自动标题、短 id、排序。
- 隐私与交付：敏感 diff、blocked terms、support bundle、handoff blocker、verify/gate。

本次尚未完整运行全量测试；只是读取了测试并执行了轻量本地命令。建议最终检查时运行：

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
./scripts/deepcli selftest --json
./scripts/deepcli round --json --run-benchmark --fail-on-command
```

## 7. 当前限制与待确认点

这些不是“未实现入口”，而是需要人工按产品目标继续确认的边界：

1. 真实 Agent 端到端质量未由本次盘点验证
   本次没有发起真实 provider 编程任务，也没有验证模型在复杂仓库中的自动修改、测试失败修复、长任务续跑质量。

2. Benchmark 证据当前已过期
   当前 required benchmark artifact 都是 2026-06-08 左右生成，当前检查日为 2026-06-26，`round` 和 `benchmark status` 都要求刷新。

3. DeepSeek/Kimi 以外 provider 尚未完整实现
   当前 provider factory 只接受 `deepseek` 和 `kimi` 类型；OpenAI、Anthropic、本地模型是后续扩展方向。

4. GitHub/PR 远程协作未纳入当前本地实现闭环
   本地 Git inspect/write/handoff 已有，但远程 issue/PR 创建、review comment 处理不是当前实现重点。

5. 语义索引、自动升级、组织级权限、MCP/IDE/浏览器/桌面自动化仍属于非目标或后续版本
   需求文档明确本阶段不做部分能力，技术计划也将它们放在增强方向。

6. `src/commands.rs` 体量很大
   当前命令实现高度集中在单文件，功能密度高、测试多，但长期维护上可能需要按领域拆分；本次只做盘点，不建议顺手重构。

## 8. 人工检查建议

建议按以下顺序检查：

1. 先看命令面是否符合预期：

```bash
./scripts/deepcli --help
./scripts/deepcli completion json | python3 -m json.tool
```

2. 再看本地产品循环状态：

```bash
./scripts/deepcli scorecard --json | python3 -m json.tool
./scripts/deepcli round --json | python3 -m json.tool
./scripts/deepcli benchmark status --json | python3 -m json.tool
./scripts/deepcli recipes sota --json | python3 -m json.tool
```

3. 刷新当前过期 benchmark 证据：

```bash
./scripts/deepcli round --json --run-benchmark --fail-on-command
```

4. 检查安全与发布门禁：

```bash
./scripts/deepcli privacy --json --fail-on-findings
./scripts/deepcli preflight --json
./scripts/deepcli gate --json
```

5. 检查核心交互能力：

```bash
./scripts/deepcli
./scripts/deepcli resume --dry-run --json
./scripts/deepcli fork --dry-run --json
./scripts/deepcli terminal --dry-run --json
```

6. 检查本地库能力：

```bash
./scripts/deepcli prompt list --json
./scripts/deepcli skill list --json
./scripts/deepcli agent list --json
```

## 9. 本次盘点结论

当前实现已经覆盖 deepcli 作为 local-first AI coding CLI 的主要本地产品骨架：命令发现、交互 UI、Provider、工具、权限、会话、恢复、fork、验收、诊断、隐私、benchmark 和产品循环都有真实代码与测试证据。

下一轮最高优先级不是继续扩展命令数量，而是用当前产品循环刷新证据并做真实端到端 Agent 任务验收：

```bash
./scripts/deepcli round --json --run-benchmark --fail-on-command
./scripts/deepcli preflight --json
./scripts/deepcli gate --json
```

如果这些通过，再选择一个真实代码改动任务，用 TUI、provider、工具调用、测试修复、handoff 走完整链路，验证“功能已实现”是否转化成“日常编程体验可靠”。
