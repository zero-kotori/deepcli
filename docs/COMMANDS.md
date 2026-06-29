# deepcli 命令分组

本文件是 harness 重构阶段 0 的命令分组基线。当前兼容策略偏保守：保持现有公开 slash 命令可用，但把主文档和后续实现导向 `core` 分组。`support`、`legacy`、`experimental` 命令应保持精简，不应成为新的所有权中心。

| 命令 | 分组 | Owner | 状态 | 说明 |
|---|---|---|---|---|
| /help | support | commands | stable | 命令发现与主题帮助。 |
| /version | support | commands | stable | 本地元数据与 support 报告。 |
| /about | legacy | commands | stable alias | `/version` 的别名。 |
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
| /health | support | commands | stable alias | doctor 与环境检查的快捷方式。 |
| /diagnose | support | commands | stable | 工作区与会话诊断。 |
| /support | support | commands | stable | 生成脱敏 support bundle。 |
| /doctor | support | commands | stable | 本地 setup 与环境诊断。 |
| /trace | core | commands | stable | 会话审计事件检查。 |
| /logs | support | commands | stable | 脱敏日志检查。 |
| /privacy | core | commands | stable | 隐私与敏感值扫描。 |
| /context | support | commands | stable | 工作区上下文预览。 |
| /permissions | core | permissions | stable | 权限模式检查。 |
| /login | support | commands | stable alias | 凭证设置快捷方式。 |
| /auth | legacy | commands | stable alias | 凭证设置别名。 |
| /apikey | legacy | commands | stable alias | 凭证设置别名。 |
| /key | legacy | commands | stable alias | 凭证设置别名。 |
| /logout | support | commands | stable alias | 凭证移除快捷方式。 |
| /credentials | core | commands | stable | Provider 凭证管理。 |
| /config | core | commands | stable | 有效配置检查与编辑。 |
| /timeout | support | commands | stable | Provider-turn 超时快捷方式。 |
| /model | core | commands | stable | Provider/model 检查与切换。 |
| /goal | core | session | stable | 长期目标契约与 gate。 |
| /plan | core | session | stable | 需求澄清与计划草稿。 |
| /fork | core | session | stable | 持久化上下文复制与恢复验证。 |
| /diff | core | commands | stable | 工作区或会话 diff 检查。 |
| /review | core | commands | stable | 本地 diff 风险审查。 |
| /accept | core | commands | stable alias | 基于 `/verify` 的人工验收报告。 |
| /gate | core | commands | stable alias | 基于 `/verify` 的严格验证 gate。 |
| /verify | core | commands | stable | 验收报告与阻断项汇总。 |
| /handoff | core | commands | stable | 交接与可提 PR 的报告。 |
| /test | core | tools | stable | 经工具层做测试发现与执行。 |
| /env | core | tools | stable | 环境 check/plan/setup/test 工作流。 |
| /check | legacy | tools | stable alias | `/env check` 的别名。 |
| /docker | legacy | tools | stable alias | 目标优先的 `/env` 别名。 |
| /compiler | legacy | tools | stable alias | 目标优先的 `/env` 别名。 |
| /setup | legacy | tools | stable alias | `/env setup` 的别名。 |
| /install | legacy | tools | stable alias | `/env install` 的别名。 |
| /git | core | tools | stable | Git 检查与受控写操作。 |
| /web | support | tools | stable | 经权限检查的 web 搜索。 |
| /prompt | support | commands | stable | 本地 prompt 库。 |
| /skill | support | commands | stable | 本地 skill 库。 |
| /agent | support | commands | stable | 子 agent 任务描述。 |
| /btw | core | session | stable | 旁路问题队列。 |
| /approval | core | session | stable | 审批队列检查与处理。 |
| /session | core | session | stable | 持久化会话检查与维护。 |
| /history | legacy | session | stable alias | `/session list` 的别名。 |
| /cleanup | legacy | session | stable alias | `/session prune-empty` 的别名。 |
| /next | support | session | stable | 可能的下一步动作报告。 |
| /resume | core | session | stable | 会话恢复与候选预览。 |
| /rename | legacy | session | stable alias | 运行时会话标题重命名。 |
| /stop | core | runtime | stable | 停止活动 TUI 任务并保持会话可恢复。 |
| /quit | core | ui | stable | 退出交互会话。 |
| /terminal | core | tools | stable | 打开或预览同工作区终端。 |

兼容性说明：

- `legacy` 表示“在未明确移除前保持兼容”，并不代表当前已损坏或废弃。
- 除非直接服务于核心 harness 重构或已确认的核心能力，否则冻结新增的细碎 slash 命令与顶层别名。
- 稳定 JSON schema 应保持现有版本，除非在同一次改动中加入迁移计划和测试。
- running-safe 状态仍以代码为准；本文档记录的是分组与所有权，不是运行时 TUI 分发行为。
