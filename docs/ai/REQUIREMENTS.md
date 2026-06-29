# deepcli 需求文档（高层）

> 本文只保留产品愿景、范围边界、权限模型和仍有效的核心约束/验收标准。具体命令清单、JSON schema、各页面交互、模块行为等"会随实现漂移"的细节不再写在这里，改由权威文档维护：
> - 命令分组与所有权：`docs/COMMANDS.md`
> - 核心功能契约：`docs/CORE_FEATURES.md`
> - 架构与模块地图：`docs/ARCHITECTURE.md`、`docs/HARNESS.md`、`docs/MODULES/*.md`
> - 当前阶段决策与 handoff：`docs/ai/CONTEXT.md`
> - 不可逆架构决策：`docs/ADR/*.md`

## 1. 产品愿景

`deepcli` 是面向 CLI 用户的开源 AI 编程代理工具，以 DeepSeek-compatible provider 为核心并预留多 provider 能力。目标是对齐 Claude Code、Codex CLI 一类 SOTA 编程 CLI 的核心能力：代码理解、自动修改、多轮执行、工具调用、审批流、安全沙箱、长任务续跑、Git 工作流、Skill/子 Agent、`/` 指令和完整任务循环。

差异化由本项目独立实现：核心架构、工具执行、权限控制、会话管理、Skill 系统、命令系统和配置系统都不复制现有工具源码。优先支持 macOS；本地开发阶段不做发布渠道，后续开源到 GitHub。

## 2. MVP 范围（已达成的本地骨架）

MVP 不是最小聊天 CLI，而是具备完整编程代理闭环的可用版本，覆盖：

- 混合 CLI 交互：一次性任务、流式 one-shot、TUI（message box + 任务观察面板）、`/` 指令，以及旧版 `repl` 兼容入口。
- Provider 接入：DeepSeek（OpenAI 兼容）与 Kimi（Anthropic 风格），含流式、tool call、JSON、usage、重试与代理；其余 provider 预留。
- 仓库理解：工作区授权、上下文读取、ignore 与敏感文件过滤。
- Agent 循环：分析 → 计划 → 修改 → 测试 → 修复 → 汇报，及 review/Git 提交的扩展工作流。
- 工具系统：文件、shell、Git、测试、环境、web、terminal、prompt、skill、子 Agent，统一经权限层。
- 权限与 sandbox：默认 sandbox，读允许、系统写与危险命令受限；低风险自动审批，高风险升级用户审批；工具调用全生命周期审计、输出脱敏。
- 会话：持久化消息、工具调用、审计、plan、goal、diff、backup、审批与旁路问题；支持恢复、fork、短 id。
- 验收与交付：diff/review/verify/handoff、preflight、gate。
- 产品循环：scorecard、round、benchmark、recipes、opportunities，输出稳定 JSON（`nextActions` + `checklist[]`）。
- 诊断与支持：version、doctor、diagnose、support bundle、logs、trace、privacy 扫描。
- 本地库：prompt、skill、agent 的查看与基础 CRUD/渲染。

## 3. 完整版本增强方向

- 成熟多 provider 生态（OpenAI、Anthropic、本地/兼容服务）。
- 更完整的 TUI：任务观察各视图消费领域 projection（UI 收束方向见 `docs/ARCHITECTURE.md`）。
- 多 Agent 协作、长任务调度与回放。
- GitHub 远程协作（issue/PR 上下文、创建 PR、review comment）。
- 本地语义索引、自动升级与配置迁移、企业代理/私有网关。
- benchmark 体系：与同类工具在完成率、耗时、成本上的对比。

## 4. 非目标（本阶段）

- 不复制同类工具源码；不做 IDE 插件、浏览器/桌面自动化、截图理解。
- 不做 MCP/Agents SDK/LangGraph 等生态协议适配。
- 不做遥测、组织级权限/审计。
- 不做发布链路（npm/pip/Homebrew/Cargo/二进制分发）。
- 不要求远程 PR；当前验收只需本地 Git 仓库。
- 上下文压缩重构与 LLM wiki 暂不在当前实现范围，待架构重构完成后单独立项。

## 5. 用户角色与权限模型

角色：CLI 用户（发起任务、审批、查看结果）、Agent（分析/计划/修改/测试/汇报）、Auto-reviewer（自动审低风险、不确定时升级）、子 Agent（受最大深度约束）。

权限模式：只读 / 写（遵循 diff 与审批）/ 完全控制（高风险仍二次确认）/ sandbox（默认，超出能力时升级 auto-reviewer 或用户审批）。

必须二次确认的高风险操作：递归删除（`rm -rf`）、`git reset --hard`、系统目录写入、修改主目录敏感配置、批量删除、安装/升级系统级依赖、推送远程或创建远程 PR。

权限决策的实现细节见 `src/permissions.rs` 与 `docs/MODULES/permissions.md`。

## 6. 仍有效的核心约束

- 命令面收束为核心 + support/legacy；新增细碎 slash 命令默认冻结（见 `docs/COMMANDS.md`）。
- 稳定 JSON schema 由 `src/schema_ids.rs` 统一拥有；公开输出尽量提供 `report` + `nextActions` + `checklist[]`，`nextActions` 必须是可直接执行的 `deepcli ...` 命令。
- 工具写入/shell/Git/网络/Docker 等操作必须经权限层；会话只经 `SessionStore` 修改。
- 模块边界与文档同步规则见 `docs/HARNESS.md`；行为变更需同步对应文档。

## 7. 验收标准

- 在 macOS 当前目录启动并进入交互式会话；读取 `.deepcli/config.*` 与 provider 凭据；完成 provider 流式调用。
- 申请目录读取权限并遵守 ignore；Agent 默认 sandbox，缺权限时经 auto-reviewer 或用户 approval 升级。
- 生成任务计划并执行完整编程循环：改文件、展示 diff、运行测试、修复失败。
- 保存与恢复会话；`/status` 展示 token/上下文/任务状态。
- 管理 prompt、生成/调用 Skill、按最大深度 spawn 子 Agent、执行受控 Git 工作流、对高风险命令二次确认。
- 端到端目标：以本 CLI 为入口，完成一个真实编译器项目从需求到 Docker 自动化测试通过的流程。
- 不泄露本地 API Key、隐私文件和被 ignore 的内容。

## 8. 风险与待确认

- MVP 范围大，需按里程碑拆分。
- provider 的 tool calling/JSON/缓存/token 统计以实际 API 能力为准。
- 自动审批与完全控制权限默认保守，避免安全风险。
- 长任务续跑依赖稳定状态机与会话格式；子 Agent 并发需文件写入锁或任务所有权。
