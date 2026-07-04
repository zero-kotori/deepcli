# deepcli 命令分组

本文件是 harness 重构阶段 0 的命令分组基线。当前兼容策略偏保守：保持现有公开 slash 命令可用，但把主文档和后续实现导向 `core` 分组。`support`、`legacy`、`experimental` 命令应保持精简，不应成为新的所有权中心。

| 命令 | 分组 | Owner | 状态 | 说明 |
|---|---|---|---|---|
| /help | support | commands | stable | 命令发现与主题帮助。 |
| /version | support | commands | stable | 本地元数据与 support 报告。 |
| /quickstart | support | commands | stable | 首次运行引导与 setup 检查。 |
| /recipes | support | commands | stable | 工作流目录；SOTA recipe 作为导航辅助。 |
| /scorecard | core | commands | stable | 产品能力评分。 |
| /opportunities | experimental | commands | stable | 非阻断的机会点报告。 |
| /benchmark | support | commands | stable | 本地 benchmark 证据管理；细分子命令保持 support。 |
| /round | core | commands | stable | 主产品循环 gate 报告。 |
| /selftest | support | commands | stable | 产品自检。 |
| /preflight | core | commands | stable | 发布与检查点 preflight。 |
| /completion | support | commands | stable | Shell 补全与命令目录。 |
| /init | support | commands | stable | 项目初始化助手。 |
| /status | core | commands | stable | 活动会话与工作区状态。 |
| /usage | core | commands | stable | Provider 与会话用量诊断。 |
| /diagnose | support | commands | stable | 工作区与会话诊断。 |
| /support | support | commands | stable | 生成脱敏 support bundle。 |
| /doctor | support | commands | stable | 本地 setup 与环境诊断。 |
| /trace | core | commands | stable | 会话审计事件检查。 |
| /logs | support | commands | stable | 脱敏日志检查。 |
| /privacy | core | commands | stable | 隐私与敏感值扫描。 |
| /context | support | commands | stable | 工作区上下文预览。 |
| /permissions | core | permissions | stable | 权限模式检查。 |
| /login | support | commands | stable alias | 凭证设置快捷方式。 |
| /apikey | legacy | commands | stable alias | 凭证设置别名。替代：`/credentials set`。 |
| /logout | support | commands | stable alias | 凭证移除快捷方式。 |
| /credentials | core | commands | stable | Provider 凭证管理。 |
| /config | core | commands | stable | 有效配置检查与编辑。 |
| /timeout | support | commands | stable | Provider-turn 超时快捷方式。 |
| /model | core | commands | stable | Provider/model 检查与切换。 |
| /goal | core | session | stable | 长期目标契约与 gate。 |
| /plan | core | session | stable | 模型驱动的只读规划与定制问题。 |
| /fork | core | session | stable | 持久化上下文复制与恢复验证。 |
| /diff | core | commands | stable | 工作区或会话 diff 检查。 |
| /review | core | commands | stable | 本地 diff 风险审查。 |
| /accept | core | commands | stable alias | 基于 `/verify` 的人工验收报告。 |
| /gate | core | commands | stable alias | 基于 `/verify` 的严格验证 gate。 |
| /verify | core | commands | stable | 验收报告与阻断项汇总。 |
| /handoff | core | commands | stable | 交接与可提 PR 的报告。 |
| /test | core | tools | stable | 经工具层做测试发现与执行。 |
| /compiler | legacy | tools | stable | 编译环境的 check/plan/setup/install/test（目标优先）。替代：`/doctor compiler` 用于诊断，setup 仍走受控环境流程。 |
| /install | legacy | tools | stable | 准备本地 docker 或 compiler 环境。替代：`/doctor compiler` 先诊断，再按报告执行环境 setup。 |
| /git | core | tools | stable | Git 检查与受控写操作。 |
| /web | support | tools | stable | 经权限检查的 web 搜索。 |
| /prompt | support | commands | stable | 本地 prompt 库。 |
| /skill | support | commands | stable | 本地 skill 库。 |
| /agent | support | commands | stable | 子 agent 任务、运行、恢复与日志观察。 |
| /btw | core | session | stable | 旁路问题队列。 |
| /approval | core | session | stable | 审批队列检查与处理。 |
| /session | core | session | stable | 持久化会话检查与维护。 |
| /cleanup | legacy | session | stable alias | `/session prune-empty` 的别名。替代：`/session prune-empty`。 |
| /resume | core | session | stable | 会话恢复与候选预览。 |
| /rename | legacy | session | stable alias | 运行时会话标题重命名。替代：`/session rename --current`。 |
| /stop | core | runtime | stable | 停止活动交互任务并保持会话可恢复。 |
| /quit | core | ui | stable | 退出交互会话。 |
| /terminal | core | tools | stable | 打开或预览同工作区终端。 |
| /cmd | core | tools | stable | 在当前 workspace 执行受控 shell 命令；`--attach` 可把输出作为下一条模型上下文。 |

## 删除/降级审计

当前未发现可直接删除的公开入口；重复或低频入口已经收束为 parser thin alias、legacy successor/policy 或 completion-only alias metadata。后续如果删除任一公开入口，必须同步更新 registry、parser、completion catalog 和本节审计记录。

Parser thin alias 决策：

- `/support` -> `/diagnose`：保留 support 别名，用于脱敏 support bundle 快捷入口。
- `/login` -> `/credentials`：保留 support 别名，用于凭证设置快捷入口。
- `/apikey` -> `/credentials`：保留 legacy 别名；替代入口是 `/credentials set`。
- `/logout` -> `/credentials`：保留 support 别名，用于凭证移除快捷入口。
- `/accept` -> `/verify`：保留 core 别名，用于人工验收语义。
- `/gate` -> `/verify`：保留 core 别名，用于严格验证 gate 语义。
- `/compiler` -> `/env`：保留 legacy 目标优先环境入口；替代入口是 `/doctor compiler` 诊断后按报告执行环境流程。
- `/install` -> `/env`：保留 legacy 环境安装入口；替代入口是 `/doctor compiler` 诊断后按报告执行环境流程。
- `/cleanup` -> `/session`：保留 legacy 会话清理别名；替代入口是 `/session prune-empty`。

Legacy slash 决策：

- `/apikey` -> `/credentials set`：凭证设置兼容别名，继续降级展示。
- `/compiler` -> `/doctor compiler`：历史环境入口，继续降级展示。
- `/install` -> `/doctor compiler`：历史环境安装入口，继续降级展示。
- `/cleanup` -> `/session prune-empty`：历史清理入口，继续降级展示。
- `/rename` -> `/session rename --current`：历史活动会话改名入口，继续降级展示。

Completion-only alias 决策：

- `completion:deepseek`：保留 support provider preset。
- `completion:kimi`：保留 support provider preset。
- `completion:ask`：保留 support one-shot alias。
- `completion:stream`：保留 support streaming one-shot alias。
- `completion:tui`：保留 support 原生终端聊天兼容 alias。
- `completion:repl` -> `tui`：保留 legacy 原生终端聊天兼容 alias，降级展示并指向 `tui`。
- `completion:sessions`：保留 support `session list` alias。
- `completion:completions`：保留 support `completion` alias。

兼容性说明：

- `legacy` 表示“在未明确移除前保持兼容”，并不代表当前已损坏或废弃。
- `deepcli completion json` 会输出 `groups[]` 和 `legacyCommands[]`，其中 `legacyCommands[]` 来自 registry 的 successor/policy metadata，覆盖 slash legacy 命令和 completion-only legacy alias；外部 UI 应使用这些字段把 legacy 入口降级展示，并指向替代命令。
- 除非直接服务于核心 harness 重构或已确认的核心能力，否则冻结新增的细碎 slash 命令与顶层别名。
- 稳定 JSON schema 应保持现有版本，除非在同一次改动中加入迁移计划和测试。
- running-safe 状态仍以代码为准；本文档记录的是分组与所有权，不是运行时交互分发行为。
