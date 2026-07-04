# deepcli 核心功能契约

本文记录核心功能的稳定契约。命令分组见 `docs/COMMANDS.md`，架构见 `docs/ARCHITECTURE.md`，模块细节见 `docs/MODULES/*.md`。稳定 JSON schema 标识符由 `src/schema_ids.rs` 拥有。

## CLI 与命令发现

- 默认 `deepcli` 进入原生终端聊天；`deepcli tui` / `deepcli repl` 为兼容 alias；`deepcli ask|stream <prompt>` 为 one-shot（缺 prompt 本地报错）。
- 高频 slash 命令、provider 前缀（deepseek/kimi）与模式有顶层别名；`--help`/`-h` 转发到 `/help <topic>`。
- 本地 one-shot 命令不创建空 session、不预调用 provider；未知/拼错命令本地拦截并给 nearest-command 建议。
- `completion` 输出 bash/zsh/fish 脚本与 JSON 命令目录。

## Agent 与工具

- Agent 循环：分析 → 计划 → 修改 → 测试 → 修复 → 汇报。
- 工具注册表覆盖文件、shell、Git、测试、环境、web、terminal、prompt、skill、子 Agent；工具声明拥有 provider schema 与权限请求。
- 所有写入/shell/Git/网络/Docker/终端/setup 操作经权限层；工具调用全生命周期审计、输出脱敏。
- `/cmd <bash command>` 复用本地 `run_shell` 工具在当前 workspace 执行命令并把 command/exit code/stdout/stderr 回显到 UI；默认不调用 provider，`/cmd --attach <bash command>` 会把格式化输出作为下一条用户上下文交给模型。

## 权限与 sandbox

- 默认 sandbox：工作区读允许，系统写与危险命令受限。
- 风险分级：只读 shell 可允许；测试/构建类可自动审批；Docker、依赖安装、破坏性 shell/Git 需审批或二次确认。

## 会话、恢复、fork、goal

- 会话持久化 metadata/messages/tool calls/audit/plan/goal/diff/backup/审批/旁路问题；短 id 前缀解析。
- `resume`/`fork` 默认跳过空会话、工具/诊断型会话、低信息会话、其它 workspace 会话；支持 dry-run/JSON 预览。
- `goal` 创建目标契约与 gate；`plan` 输出需求澄清草稿与旁路问题。

## 验收与交付

- `diff`/`review`/`verify`/`handoff` 聚合 diff、测试、环境、审批、旁路问题、失败工具。
- `preflight` 串联 fmt/diff-check/clippy/selftest/doctor/privacy/gate（支持 dry-run/quick/json）。
- `git` 读为只读 JSON inspect，写（create-branch/commit）为受控入口，支持 dry-run 预览。

## 产品循环

- `scorecard`/`round`/`benchmark`/`recipes`/`opportunities`：本地评分、迭代 gate、benchmark 证据管理、工作流目录、非阻塞机会。
- 关键 JSON 输出 `report` + `nextActions`（可直接执行的 `deepcli ...`）+ `checklist[]`，供原生终端聊天和外部 UI 直接渲染。

## 诊断、隐私与本地库

- `version`/`doctor`/`diagnose`/`support`/`logs`/`trace`/`privacy`：版本元数据、健康检查、脱敏支持包、审计、隐私扫描。
- `prompt`/`skill`/`agent`：本地库 list/get/render/run、子 Agent 任务持久化、后台运行、恢复与日志观察。

## 稳定 JSON schema 约定

- 标识符形如 `deepcli.<name>.v1`，由 `src/schema_ids.rs` 统一拥有；生产发射点引用常量，测试断言保留字面量作独立值锚点。
- 带 `--output` 的命令做 workspace-contained path 校验，拒绝路径逃逸。
