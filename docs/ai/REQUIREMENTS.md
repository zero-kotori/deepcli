# deepcli 当前范围与约束

本文记录当前仍有效的产品范围、边界和验收标准。它不是旧版愿景清单，也不再维护未落地的竞品对齐目标。具体命令清单、JSON schema、模块职责和架构边界由下列文档维护：

- 命令分组与所有权：`docs/COMMANDS.md`
- 核心功能契约：`docs/CORE_FEATURES.md`
- 架构与模块地图：`docs/ARCHITECTURE.md`、`docs/HARNESS.md`、`docs/MODULES/*.md`
- 当前交接上下文：`docs/ai/CONTEXT.md`
- 架构决策：`docs/ADR/*.md`

## 产品定位

deepcli 是一个 local-first 的 AI 编程代理 CLI。它以当前工作区为中心，让用户在终端里完成代码任务、会话恢复、受控工具调用、测试验证、交付报告和本地诊断。

## 当前已实现范围

- CLI 入口：原生终端聊天、one-shot `ask`/`stream`、provider 前缀、顶层命令别名和本地命令路由。
- Provider：DeepSeek-compatible Provider 为主，Kimi 与其它兼容 Provider 通过统一配置和适配接口扩展。
- Agent Runtime：上下文准备、tool-capable 流式 Provider turn、工具调用循环、真实工具成功状态、会话观测、计划与结果输出。
- 工具系统：文件、shell、Git、测试、环境、web、terminal、prompt、skill、子 Agent，统一通过 capability-only Provider schema、参数校验和 host-owned 权限层。
- 权限与安全：工作区授权、canonical/symlink 与 DeepIgnore 过滤、应用层 sandbox 策略、高风险操作识别、精确调用审批、shell 子进程凭据清洗、`run_shell`/`run_tests` 超时、凭据脱敏与隐私扫描。
- 会话：持久化消息、工具调用、审计、plan、goal、diff、backup、审批和旁路问题；支持 resume、搜索、诊断、fork 和短 id。
- 验收与交付：`test`、`diff`、`review`、`verify`、`handoff`、`preflight`、`gate`、Git inspect/write dry-run。
- 本地健康与证据：`selftest`、`doctor`、`diagnose`、`support`、`logs`、`trace`、`privacy`、`scorecard`、`round`、`benchmark`、`recipes`、`opportunities`。
- 本地库：prompt、skill、agent 的查看、渲染、运行和任务记录。

## 非目标

- 不把未验证的 demo、模拟实现或伪 Provider 当作产品行为。
- 不在当前文档中保留“完整实现所有旧需求”“对齐某个竞品水平”“完成外部编译器项目”等开放式目标。
- 不做 IDE 插件、浏览器/桌面自动化、截图理解、组织级遥测或远程审计。
- 不把发布渠道、远程 PR 工作流或企业网关作为当前默认交付要求。
- 不新增细碎 slash 命令，除非它们直接服务已有核心能力、命令收束或验证闭环。

## 权限模型

角色包括 CLI 用户、Agent、auto-reviewer 和子 Agent。权限模式分为只读、写入、完全控制和 sandbox；默认行为应保守，所有写入、shell、Git、网络、Docker、终端和 setup 动作必须经过工具系统与权限层。Provider schema 只表达操作意图，不能提交 `approved`、文件写入或联网等授权事实；这些事实由 host 从已解析调用推导。

人工审批必须绑定工具名和经 host 解析的有效参数 canonical JSON digest，普通操作需要一次确认，高危操作需要两次确认；批准只授权一次完全匹配的调用，消费后不可重放。没有 digest 的旧审批记录不得授权执行。`run_tests` 只能选择当前工作区发现的测试命令或安全扩展，不能借测试入口执行任意 shell。

必须二次确认或升级审批的操作包括：递归删除、`git reset --hard`、系统目录写入、修改敏感配置、批量删除、安装或升级系统级依赖、推送远程、创建远程 PR。

权限实现见 `src/permissions.rs` 与 `docs/MODULES/permissions.md`。

当前 `sandbox` 只实现应用层路径、风险与网络策略，不是 OS 级 shell/network 隔离。shell 超时会回收直接 child，但完整 process-group 后代取消尚未完成。子 Agent allowed-tools、canonical read/write scope、父 capability 子集和 host-owned depth 必须在 runtime/executor 强制；存在 scope 时无法安全限定的广域工具必须拒绝。`autoReviewer` 默认关闭，不能表述为可信代码执行边界。

## 当前验收标准

- `deepcli` 能在 macOS 当前目录进入原生终端聊天。
- 本地 one-shot 命令不创建空会话、不误调用 Provider。
- Provider 凭据、模型配置和超时可本地检查、设置和脱敏展示。
- Agent 修改代码时只能通过工具层和权限层访问文件、shell、Git、网络或环境。
- 需要批准的模型工具调用只能由 host 创建精确、单次的授权；工具非零退出、测试失败和超时必须以失败状态进入 Provider、审计与 UI。
- 会话可保存、恢复、搜索、诊断和 fork；dry-run JSON 不调用 Provider。
- 交付前可生成 diff/review/verify/handoff/preflight/gate 证据。
- 稳定 JSON 输出使用 `report`、可执行 `nextActions` 和 `checklist[]`，schema id 由 `src/schema_ids.rs` 统一拥有。
- 隐私扫描不泄露 API Key、凭据文件、会话日志或被 ignore 的内容。

## 维护约束

- 行为改动必须同步对应权威文档。
- 命令分组、legacy 策略和公开入口变更必须同步 `docs/COMMANDS.md` 并跑对应契约测试。
- 模块职责变化必须同步 `docs/HARNESS.md` 和 `docs/MODULES/*.md`。
- 提交前按变更范围运行最小测试；涉及发布或提交前检查时运行 `deepcli preflight`。
