# deepcli 功能介绍

本文记录当前已经落地并可验收的主要能力。命令分组、架构边界和稳定 JSON 约定分别见 `docs/COMMANDS.md`、`docs/ARCHITECTURE.md` 和 `docs/CORE_FEATURES.md`。

## 产品定位

deepcli 是一个 local-first 的 AI 编程代理 CLI。它围绕当前工作区运行，让用户在终端中启动 AI 编程任务、调用受控工具、恢复会话、运行测试、检查交付状态并生成本地诊断证据。

## 启动与任务入口

- `deepcli`：进入原生终端聊天。
- `deepcli repl`：兼容 alias，进入同一原生终端聊天。
- `deepcli ask <prompt>`：一次性任务。
- `deepcli stream <prompt>`：流式一次性任务。
- `deepcli deepseek ...` / `deepcli kimi ...`：使用对应 provider 预设。
- `deepcli help` / `deepcli <command> --help`：查看命令帮助。
- `deepcli completion json`：输出机器可读命令目录。

原生终端聊天不使用 fullscreen alternate screen。输出写入普通 stdout，终端原生 scrollback、复制和滚动行为保持可用。

## Provider、模型与凭据

- `deepcli model show|list|set`
- `deepcli provider [provider] [model]`
- `deepcli use <provider> [model]`
- `deepcli switch <provider> [model]`
- `deepcli credentials status [provider] --json`
- `deepcli login <provider> --stdin --force`
- `deepcli logout <provider>`
- `deepcli credentials template|import-env|set|remove`

凭据命令在本地执行，不需要创建会话或调用 Provider。输出会脱敏，不打印明文 API key。

## Agent、工具与权限

Agent Runtime 负责上下文准备、Provider turn、工具调用循环和会话观测。Agent 不直接访问文件系统、shell、网络或 Git；所有动作都通过工具声明、参数校验和权限层。

当前工具覆盖：

- 文件读取/写入
- shell 命令
- Git inspect/write dry-run 与受控写操作
- 测试发现与执行
- 环境检查、规划和 setup
- web 搜索
- 同目录终端预览/打开
- prompt、skill、子 Agent
- 本地命令旁路 `/cmd`

权限层提供工作区授权、sandbox、风险分级、审批队列、危险命令识别和脱敏审计。

## 会话、恢复与分支

- `deepcli resume [session_id]`
- `deepcli resume [session_id] --dry-run --json`
- `deepcli resume candidates --json`
- `deepcli sessions --all --limit 20`
- `deepcli session list|show|history|summary|tools|tests|diffs|backups --json`
- `deepcli session search <query> --json`
- `deepcli session next|diagnose --json`
- `deepcli fork [session_id|--current] [--dry-run|--no-open] [--verify] --json`
- `deepcli terminal [--dry-run|--no-open] [--json]`

会话持久化 metadata、消息、工具调用、审计、plan、goal、diff、backup、审批和旁路问题。`resume` 和 `fork` 默认过滤空会话、诊断型会话和低信息会话；dry-run 只读取本地状态，不调用 Provider。

## 规划、目标与协作队列

- `deepcli goal [objective...] --json`
- `deepcli goal edit|pause|resume|complete|block|clear --json`
- `deepcli goal status|gate --json`
- `deepcli goal start "..." --token-budget <tokens>`
- `/plan <rough requirement>`
- `/plan show`
- `deepcli approval list|approve|deny|clear --json`
- `deepcli btw ask|list|answer|clear --json`

`goal` 记录当前会话目标、状态、token budget 和验收 gate；支持 pause/resume/edit/complete/block/clear 生命周期控制，`complete` 只有在 readiness gate 通过后才会落盘。`plan` 用 active Provider 读取上下文并生成实现计划。审批和旁路问题都写入会话，JSON 输出提供可执行 next actions 与 `checklist[]`。

## 验收、交付与 Git

- `deepcli test discover|run --json`
- `deepcli diff --json`
- `deepcli review --json`
- `deepcli verify --json`
- `deepcli handoff --format pr`
- `deepcli preflight [--dry-run|--quick] --json`
- `deepcli gate --json`
- `deepcli git status|diff|branch|message --json`
- `deepcli git create-branch <name> --dry-run --json`
- `deepcli git commit <message> --dry-run --json`

交付命令聚合 diff、测试、环境、审批、旁路问题和失败工具。`preflight` 串联格式检查、diff whitespace、clippy、自检、doctor、隐私扫描和 gate；`--quick` 用于本地快速迭代。

## 本地健康、证据与工作流目录

- `deepcli selftest --json`
- `deepcli doctor --quick --json`
- `deepcli diagnose --json`
- `deepcli support .deepcli/support/latest --json`
- `deepcli logs --json`
- `deepcli trace --limit 30`
- `deepcli privacy --json`
- `deepcli scorecard --json`
- `deepcli round --json`
- `deepcli benchmark presets|run-suite|run|record|status|gate|summary|trends|baseline-template|compare|baselines|list|show|clean --json`
- `deepcli recipes [topic] --json`
- `deepcli opportunities --json`

这些命令用于本地自检、诊断、证据采集和工作流导航。`recipes sota` 是历史兼容主题名，对应当前产品证据/benchmark 工作流；文档不再把它描述为开放式产品目标。

## Prompt、Skill 与子 Agent

- `deepcli prompt list|get|render|save|delete --json`
- `deepcli skill list|run|generate --json`
- `deepcli agent list|show|spawn|resume|logs --json`

本地库命令可供终端、脚本和外部 UI 使用。子 Agent 生命周期、事件日志和恢复元数据会持久化到本地。

## JSON 输出约定

公开 JSON 输出优先包含：

- `schema`：稳定 schema id
- `report`：可展示文本摘要
- `nextActions`：可直接执行的命令
- `checklist[]`：适合 UI 渲染的动作队列

带 `--output` 的命令必须经过 workspace-contained path 校验，拒绝路径逃逸。
