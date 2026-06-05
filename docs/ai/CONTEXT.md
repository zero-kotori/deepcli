# deepcli 当前对话上下文

> 持续更新中：本文件用于把当前长期产品迭代对话的关键上下文落到仓库内，方便 deepcli、Codex 或其他 agent 在新会话中继续工作。

## 当前长期目标

持续执行“产品设计师检查缺口 -> 工程师实现功能 -> 验证 -> 再次产品检查”的循环，直到 deepcli 达到 SOTA local-first AI coding CLI 水平。该目标尚未完成，不应因为某一轮通过测试或完成提交就标记为结束。

## 当前仓库状态基线

- 工作目录：当前 deepcli 仓库根目录
- 默认分支：`main`
- 远程仓库：`https://github.com/zero-kotori/deepcli.git`
- 目标提交身份：`zero-kotori <kotorizero8@gmail.com>`
- 产品命名：统一使用 `deepcli`
- 本地忽略产物：`.deepcli/benchmarks/`、`.deepcli/exports/`、credentials、logs、sessions 等不得提交

## 最近已完成的产品迭代

1. benchmark evidence 覆盖门禁
   - 提交：`aaf7845c48243deb1f313f6c646a97172b3c2b7b`
   - 结果：`benchmark status` 不再把单个 meaningful artifact 误判为 ready，必须覆盖 `cargo-test`、`preflight-quick`、`selftest`、`scorecard` 四个 required presets。
   - JSON 增加 `presetCoverage.requiredStatus`。

2. round 一键运行 benchmark suite
   - 提交：`ce8ed18c84be04ba8e15632be4116e89aee5091b`
   - 结果：`round` 默认仍为只读产品循环报告；显式传入 `--run-benchmark` 或 `--run-suite` 时，先执行 benchmark suite，再在同一份 `deepcli.round.v1` JSON 中写入 `benchmarkRun` 和更新后的 `benchmarkStatus`。
   - 推荐命令：`deepcli round --json --run-benchmark --fail-on-command`

## 当前产品自评

最近自评中，`scorecard` 为 77/80，主要缺口是当前仓库没有保留本地 benchmark evidence artifact。这个缺口是有意保留的，因为 benchmark evidence 是本地忽略产物，不应推送到远程仓库。

下一轮产品设计应继续从真实使用阻力中选一个高价值缺口，而不是只为了让分数变绿而提交本地 artifact。

## 常用检查命令

提交前建议至少运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
./scripts/deepcli preflight --json
./scripts/deepcli review
```

产品循环检查：

```bash
./scripts/deepcli scorecard --json
./scripts/deepcli round --json
./scripts/deepcli round --json --run-benchmark --fail-on-command
```

本地 artifact 清洁检查：

```bash
find .deepcli -maxdepth 2 -type f \( -path '.deepcli/benchmarks/*' -o -path '.deepcli/exports/*' \) -print
```

命名与身份扫描：

```bash
rg -n "legacy command spelling markers" -g '!target' .
git log --all --format='%H%x09%an%x09%ae%x09%s' | rg 'non-target personal identity markers'
git grep -n -I -E 'non-target personal identity markers' -- . ':!target'
```

上面两类 marker 命令应在执行时替换为本地私有扫描模式，不要把非目标个人身份字面量写入仓库文档。

## 下一步建议

- 继续检查 `docs/ai/REQUIREMENTS.md` 中尚未被当前实现充分覆盖的 SOTA 能力。
- 优先选择用户明显会感知到的阻力，例如真实 benchmark evidence 工作流、竞品对比基准、环境自动化验收、TUI 产品循环入口、或长任务可观测性。
- 每轮完成后提交并推送，保持工作区干净。
