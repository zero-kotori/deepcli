# deepcli

deepcli 是一个 local-first 的 AI 编程代理 CLI，面向日常工程协作：启动原生终端聊天、切换 Provider/模型、恢复会话、检查健康状态、准备本地环境、运行测试，以及生成验收或交付报告。

本文是快速入口。命令清单、功能契约与架构见下方 [文档](#文档)。

## 当前状态

产品仍在快速迭代中。命令面正在按 harness 重构计划收束为"核心 + support/legacy"，文档以当前已落地并可验收的能力为准。

## 快速开始

```bash
# 构建并在当前项目启动原生终端聊天
cargo build
./scripts/deepcli            # 或 deepcli（若已在 PATH）

# 本地自检，不调用 Provider
deepcli selftest --json
deepcli doctor --quick --json
deepcli doctor shell --json

# 配置凭据并查看状态
printf '%s' "$DEEPSEEK_API_KEY" | deepcli login deepseek --stdin --force
deepcli credentials status --json

# 切换 Provider / 模型
deepcli model set deepseek deepseek-v4-pro
deepcli model list --json

# 恢复历史任务
deepcli resume
deepcli resume <session_id> --dry-run --json
deepcli sessions --all --limit 20

# 长期目标 / 需求澄清 / 复制会话 / 同目录终端
deepcli goal "完整实现当前项目文档中的全部需求" --json
deepcli goal status --json
/plan 做一个可以交互式澄清需求的功能
deepcli fork --current --no-open --verify --json
deepcli terminal --dry-run --json
deepcli cmd git status --short
```

更多命令与一次性 JSON 入口见 `docs/COMMANDS.md` 与 `docs/CORE_FEATURES.md`。

## 文档

- [命令分组](docs/COMMANDS.md)：命令、分组、所有权与状态。
- [核心功能契约](docs/CORE_FEATURES.md)：稳定行为与 JSON 约定。
- [架构](docs/ARCHITECTURE.md)、[Harness](docs/HARNESS.md)、[模块说明](docs/MODULES/)：分层、边界、模块所有权。
- [功能介绍](docs/FEATURES.md)：面向用户的能力清单。
- [架构决策记录](docs/ADR/)：不可逆的架构决策。
- 设计背景：[需求](docs/ai/REQUIREMENTS.md)、[技术方案（历史）](docs/ai/TECHNICAL_PLAN.md)、[重构计划](docs/ai/HARNESS_REFACTOR_PLAN.md)。

## 本地验证

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
./scripts/deepcli selftest --json
./scripts/deepcli doctor --quick --json
./scripts/deepcli scorecard --json
./scripts/deepcli round --json
./scripts/deepcli preflight --json
./scripts/deepcli privacy --json
```

`selftest` 与 `doctor` 会对比 `.deepcli/config.json` 的 `project.gitIdentity` 与当前 Git 仓库的有效 `user.name`/`user.email`，用于提交前发现错误作者身份。

`preflight` 是提交/推送前的一键本地检查（fmt、diff whitespace、clippy、selftest、doctor、privacy、gate）；`--dry-run` 预览、`--quick` 跳过较慢的 clippy/gate。`privacy` 用于开源前的 Git 历史隐私审计，可用 `privacy.allowed*` / `privacy.blockedTerms` 配置允许项与禁用词。

## 仓库

```text
https://github.com/zero-kotori/deepcli
```
