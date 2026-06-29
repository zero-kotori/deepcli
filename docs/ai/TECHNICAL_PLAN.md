# deepcli 技术方案（原始设计方案）

> 本文是项目早期的技术设计方案，保留作为高层设计背景与里程碑历史。**当前真实架构与模块地图以 `docs/ARCHITECTURE.md`、`docs/HARNESS.md`、`docs/MODULES/*.md` 为准**；当前命令面、JSON schema 与功能契约以 `docs/COMMANDS.md`、`docs/CORE_FEATURES.md` 为准。原 §13「`/` 指令设计」的细节命令清单已移除，不再作为命令数据库维护。

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

CLI 入口在构造 `AgentRuntime` 前先识别本地 one-shot slash 命令与顶层/provider 前缀别名，避免为只读/本地命令创建空 session 或误把命令当 prompt 发给 provider；`ask`/`stream` 缺 prompt、明显拼错的命令都在本地拦截。具体命令清单、别名映射、各命令的 JSON schema 与产品循环（scorecard/round/benchmark/recipes/opportunities）行为契约以 `docs/COMMANDS.md` 与 `docs/CORE_FEATURES.md` 为准，不在本设计文档内重复维护。

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

只读子命令 `status|diff|branch|message` 支持 `--json` 输出稳定 `deepcli.git.inspect.v1`，用于 TUI、外部 UI 和脚本读取 Git 状态而不解析纯文本。JSON 应包含 `kind`、实际执行命令、exit code、stdout/stderr、原始 raw、report、可执行 `deepcli ...` next actions，以及从这些动作派生的 `checklist[]`；`checklist[]` 每项包含 `step`、`label` 和 `command`，用于 Git 面板直接渲染 diff、commit message、review、gate 和帮助动作；`--output` 复用 workspace-contained 写文件校验，把当前选择的文本或 JSON 输出写入 artifact；`diff` 支持 `--staged|--cached`。只读子命令遇到未知 option 或多余参数时必须报错，避免脚本把被忽略参数后的空输出误判为结构化成功。

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
