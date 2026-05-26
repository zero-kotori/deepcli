# deep-cli 需求文档

## 1. 最终需求理解

`deep-cli` 是一个面向 CLI 用户的开源 AI 编程代理工具，使用 DeepSeek API 作为核心 provider，同时预留多 provider 能力。产品目标是对齐 Claude Code、Codex CLI 一类 SOTA 编程 CLI 的核心能力，包括代码理解、自动修改、多轮执行、工具调用、审批流、安全沙箱、长任务续跑、Git/PR 工作流、Skill 生成与调用、多种 `/` 指令和完整任务循环。

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
- 更完整的 TUI：展示计划、工具调用、diff、token/上下文消耗、审批、日志、任务状态。
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

- `deep-cli` 命令入口。
- 在当前目录启动 Agent。
- 支持一次性任务参数。
- 支持交互式会话。
- 支持恢复历史会话。

### 5.2 Message Box

- 支持正常 IDE 常用组合键。
- `Shift+Enter` 换行，`Enter` 提交。
- 支持多行输入、历史输入、粘贴大段文本。
- 支持 Agent 运行期间提出 by-the-way 小问题，不打断主任务。
- 支持从 message box 打开相同目录的新终端。

### 5.3 `/` 指令

建议 MVP 至少支持：

- `/help`：展示可用指令。
- `/status`：展示任务状态、token 消耗、上下文消耗。
- `/permissions`：查看或调整当前目录权限。
- `/config`：查看有效配置来源。
- `/model`：切换或查看 provider/model。
- `/plan`：查看当前计划。
- `/diff`：查看待应用或已应用修改。
- `/review`：触发 auto-reviewer 或人工 review。
- `/test`：发现并运行测试命令。
- `/env`：检测、安装、配置和验证本地任务环境，例如 Docker/Colima 和 compiler-dev 镜像。
- `/git`：查看 Git 状态或执行受控 Git 操作。
- `/prompt`：管理自定义 prompt 和内置 prompt。
- `/skill`：发现、生成、注册、调用 Skill。
- `/resume`：恢复会话。
- `/terminal`：打开同目录终端。

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
- 日志和 trace：`.deepcli/logs/`，默认不提交。

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

1. 用户在项目目录执行 `deep-cli`。
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
- message box 支持多行输入和常用 IDE 组合键。
- `/status` 可展示 token、上下文、任务状态。
- Agent 运行时可处理 by-the-way 小问题，并回到主任务。
- 用户中断时保存现场并提示恢复方式。

## 12. 技术设计建议

- 使用模块化架构，明确 CLI、TUI、Agent runtime、provider、tool、permission、session、skill、git 的边界。
- Provider 使用适配器模式，DeepSeek 为默认适配器，Kimi 和其他 provider 复用同一接口。
- 工具调用必须先经过权限引擎，不允许 Agent 直接绕过权限执行。
- 文件修改使用直接写入，但写入前后生成 diff，并记录到会话。
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

- 通过调用本项目产出的 `deep-cli` 产品，在本地 Git 仓库中启动 Agent。
- Agent 根据 `work/myWork/compiler` 项目中的需求文档和 `online-doc` 要求，独立 coding 生成完整 Rust 编译器实现代码，从 lv1 到 lv9+。
- Agent 在验收过程中可以连接 web 获取必要公开资料，但必须遵守隐私过滤和 sandbox/approval 策略。
- Agent 根据需求文档中的环境配置，独立配置 Docker 环境、拉取 image，并运行本地自动化测试。
- 验收执行期间允许调用本项目配置的 DeepSeek API，并允许使用 DeepSeek V4 Pro 作为 Agent 执行模型；实际 API model id 以 provider 配置为准。
- 如果验收过程中发现 `deep-cli` 产品能力不足、流程中断、权限策略错误、工具调用失败或 Agent 无法继续，应回到本项目修复和完善 CLI，再重新执行验收测试。
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
