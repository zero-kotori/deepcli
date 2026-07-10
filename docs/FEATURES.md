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

原生终端聊天不使用 fullscreen alternate screen。输出写入普通 stdout，终端原生 scrollback、复制和滚动行为保持可用。输入会成对启用/关闭 bracketed paste，多行粘贴进入同一编辑缓冲；Provider、工具和审批文本在写终端前移除控制序列。普通 Agent turn 统一携带对应工具 schema 并消费流式文本/工具事件，不再为短请求切换到无工具的 Provider 快速路径。默认界面使用短 session header 和 `you`/`deepcli` 角色标签，隐藏 Provider 生命周期、消息/工具数量、请求体积和成功工具进度；工具失败、审批与计划问题仍可见，完整事件继续进入会话审计。

## Provider、模型与凭据

- `deepcli model show|list|set`
- `deepcli provider [provider] [model]`
- `deepcli use <provider> [model]`
- `deepcli switch <provider> [model]`
- `deepcli credentials status [provider] --json`
- `deepcli login <provider> --stdin --force`
- `deepcli logout <provider>`
- `deepcli credentials template|import-env|set|remove`

凭据命令在本地执行，不需要创建会话或调用 Provider。输出会脱敏，不打印明文 API key。有效配置先在原始 JSON 层递归合并默认值、全局配置和项目配置，对象保留未覆盖字段，最后应用环境变量覆盖。

## Agent、工具与权限

Agent Runtime 负责上下文准备、tool-capable Provider turn、工具调用循环和会话观测。Agent 不直接访问文件系统、shell、网络或 Git；所有动作都通过工具声明、参数校验和权限层。Provider 可见 schema 只包含业务参数，`approved`、`writes_files`、`requires_network` 等授权或风险声明不属于模型输入，权限上下文由 host 根据工具和已解析参数生成。

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

权限层提供工作区授权、应用层 sandbox 策略、风险分级、审批队列、危险命令识别和脱敏审计。需要人工批准的调用按工具名和经 host 解析的有效参数 canonical JSON 计算 SHA-256 digest，批准只能匹配该调用并在执行时消费一次；高危调用需要两次确认。历史上没有 digest 的审批记录不能授予执行权限。

文件工具会 canonicalize 工作区和目标的最近既存祖先，拒绝经 symlink 逃逸，并对读、写和 patch 目标执行 DeepIgnore 检查。`run_tests` 只能运行当前工作区发现到的测试命令或其受限扩展，拒绝 shell 控制符、runner 配置覆盖和工作区外参数。shell 子进程会移除 Provider/令牌/私钥类环境变量；`run_shell` 与 `run_tests` 使用有界超时，并把超时或非零退出记录为失败的 `ToolExecution`。

`web_fetch` 在每个 DNS 解析和 redirect hop 拒绝私网、回环、链路本地、保留地址及内嵌凭据，并对 `max_chars` 和实际下载字节设置宿主上限。`git_commit` 的批准绑定暂存 tree，执行使用 `commit-tree` 加旧 HEAD compare-and-swap，不运行仓库 hooks，也不在 merge/cherry-pick/revert 中间态执行。

子 Agent 的 allowed-tools 会同时裁剪 Provider registry 和 Executor；read scope 还会过滤 system workspace context，read/write scope 在 canonical 路径解析后检查，存在 scope 时无法安全收窄的 shell/Git/test 等工具会直接拒绝。depth 由 host 从父 runtime 递增，嵌套子 Agent 不得扩大父 allowed-tools。空 scope/allowed-tools 保持未限制语义。

当前 `sandbox` 仍是应用层权限策略，不是 OS 级 shell 或 network 隔离；超时/任务结束只对直接 child 提供回收语义，尚未实现完整 process-group 后代取消。`autoReviewer` 默认关闭；显式开启只代表确定性入口校验，不代表仓库代码可信。

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

本地库命令可供终端、脚本和外部 UI 使用。子 Agent 生命周期、事件日志、声明的 scope/allowed-tools 和恢复元数据会持久化到本地；resume 时这些字段会重新装载为 runtime/executor capability。

## JSON 输出约定

公开 JSON 输出优先包含：

- `schema`：稳定 schema id
- `report`：可展示文本摘要
- `nextActions`：可直接执行的命令
- `checklist[]`：适合 UI 渲染的动作队列

带 `--output` 的命令必须经过 workspace-contained path 校验，拒绝路径逃逸。
