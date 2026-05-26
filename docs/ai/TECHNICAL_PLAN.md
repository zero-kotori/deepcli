# deep-cli 技术方案

## 1. 设计目标

`deep-cli` 采用本地优先、权限受控、可恢复的 Agent Runtime 设计。系统需要支持 DeepSeek API，并通过统一 provider 接口扩展到 Kimi、OpenAI、Anthropic 或本地模型。CLI 要能执行完整编程代理链路：理解项目、制定计划、调用工具、修改文件、运行测试、修复问题、review、保存会话、可选 Git 提交。

## 2. 总体架构

建议采用分层模块：

- `cli`：命令入口、参数解析、一次性任务、交互式入口。
- `ui`：REPL/message box/TUI 渲染、快捷键、审批交互、状态展示。
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

## 3. 推荐目录结构

```text
deep_cli/
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
- TUI 和 message box：确定采用 `ratatui + crossterm`；message box 采用自研输入组件，必须支持 `Shift+Enter` 换行、`Enter` 提交和 Agent 运行时旁路提问。
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
6. 写入会话记录。
7. 可选触发 auto-reviewer。

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
- `.deepcli/sessions/<session_id>/summary.md`

后续可迁移到 SQLite。

## 13. `/` 指令设计

`CommandRouter` 负责解析和分发：

- `/status`
- `/help`
- `/config`
- `/permissions`
- `/model`
- `/plan`
- `/diff`
- `/review`
- `/test`
- `/git`
- `/prompt`
- `/skill`
- `/resume`
- `/terminal`

所有指令都应能在 Agent 运行期间安全执行。by-the-way 小问题应进入旁路队列，不破坏主任务状态。

## 14. Prompt 和 Skill 设计

### 14.1 Prompt

- 内置常用 prompt。
- 用户自定义 prompt。
- 项目 prompt。
- 支持变量，例如当前目录、当前分支、当前文件、当前 diff。

### 14.2 Skill

Skill 由元数据和指令文件组成：

- `skill.json`
- `SKILL.md`
- 可选脚本和模板。

Skill 调用必须受权限和最大深度限制。生成 Skill 时要写入说明、触发条件、输入输出、限制和测试方式。

## 15. Git 工作流设计

MVP 支持：

- 查看状态。
- 查看 diff。
- 生成 commit message。
- 本地 commit。
- 创建分支。

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
- 最终验收必须通过调用本项目产出的 `deep-cli` 产品完成，而不是手工实现 compiler。
- 最终验收使用 `work/myWork/compiler` 的需求文档和 `online-doc` 要求，由 Agent 独立生成完整 Rust 编译器实现，从 lv1 到 lv9+。
- 最终验收允许 Agent 连接 web，允许调用 DeepSeek API，并允许使用 DeepSeek V4 Pro 作为执行模型；实际 API model id 从配置读取。
- 最终验收由 Agent 独立配置 Docker 环境、拉取 image、运行自动化测试。
- 如果验收暴露 CLI 产品缺陷，应先修复 `deep-cli`，再重新运行端到端验收。

## 20. 当前待确认

- DeepSeek V4 Pro 的实际 API model id 和接口能力细节。
- auto-reviewer 的判定策略和默认保守程度。
- 子 Agent 并发写入冲突处理方式。
- 非 Git 目录的备份和回滚策略。
