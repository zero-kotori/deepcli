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

3. 长期 goal、需求澄清 plan、会话 fork
   - 结果：新增 `/goal`、`/plan <rough requirement>` 和 `/fork` 三个产品闭环命令。
   - `/goal` 在当前 session 写入 `goal.json` 与守护 `plan.json`，并把 active goal contract 注入后续 Agent 上下文，约束 Agent 不能在目标、验收要求和测试通过前停止。
   - `/plan` 无参数保留查看执行计划；带需求文本时生成带推荐选项的需求澄清草稿，可写入 docs，并在有当前 session 时把问题加入旁路问题队列。
   - `/fork` 复制已持久化 session 上下文到新 session id，默认打开新 macOS Terminal 执行 `deepcli resume <new_id>`；当前运行中的后台 Agent 热分叉暂不宣称支持。

4. goal readiness gate
   - 结果：新增 `/goal status` 和 `/goal gate`。
   - `/goal status` 输出 `deepcli.goal.status.v1`，检查需求来源文件、goal 守护计划步骤和每条 acceptance command 的最新测试证据。
   - `/goal gate` 复用同一报告，并在仍有 blockers 时返回非零，用作“当前长期目标是否允许停止”的本地门禁。

5. one-shot goal readiness fallback
   - 结果：`deepcli goal show/status/gate` 在无 active session 或当前 session 没有 goal 时，会回退到最近一个带 goal 的会话。
   - JSON/text 输出会标注 `sessionSource`，避免用户误解门禁检查的是哪段历史。
   - 创建、启动和清理 goal 仍要求 active session，避免 one-shot 命令误写历史会话。

6. round 聚合 goal readiness
   - 结果：`deepcli round` 在存在 goal 时会自动读取最近 goal readiness，并在稳定 `deepcli.round.v1` JSON 中输出 `goalStatus` 摘要。
   - 未 ready 的 goal 会成为 `goal_readiness` gate，并把 `deepcli goal gate --json` 加入下一步动作；没有 goal 时 `round` 保持只读行为，不创建 session、不调用 Provider。
   - 目的：把 scorecard、benchmark evidence 和长期目标停止条件放进同一份产品迭代回合报告，减少每轮验收时漏跑 `/goal gate` 的风险。

7. round gate 去重与阈值语义修正
   - 结果：`deepcli round` 的 `scorecard` gate 只表示分数是否达到本轮阈值；benchmark evidence 和 goal readiness 缺口由专属 gate 呈现。
   - 当前缺少本地 benchmark artifact 时，`scorecard` gate 可通过，`benchmark_evidence` gate 失败，总体 `ready=false`，报告不再让同一个缺口重复标红。
   - 目的：让产品迭代报告更适合快速验收，避免“96% 高于阈值但 scorecard gate failed”的误导。

8. round benchmark gate 内联 required preset 摘要
   - 结果：`deepcli round` 的 `benchmark_evidence` gate summary 会直接列出 missing、weak、stale、failed 或 timeout 的 required benchmark preset。
   - 当前缺少 benchmark evidence 时，round gate 会显示 `missing presets: cargo-test, preflight-quick, selftest, scorecard`。
   - 目的：让用户在同一份产品 round 报告里知道该补哪些证据，不必先跳转到 `/benchmark status`。

9. round nextActions 失败 gate 优先
   - 结果：当 `scorecard` gate 通过且唯一剩余缺口属于 `benchmark_evidence:` 时，`deepcli round --json` 的首个 `nextActions` 是 `deepcli round --json --run-benchmark --fail-on-command`。
   - 同时省略重复的 `deepcli scorecard --json` 下一步动作，避免用户回到已经通过的 scorecard 报告。
   - 目的：让产品循环报告直接指向当前失败 gate 的修复命令，减少验收时的无效跳转。

10. scorecard nextActions gap 修复优先
   - 结果：`deepcli scorecard --json` 会把当前 gaps 的直接修复动作排在通用探索命令之前。
   - 当唯一剩余缺口属于 `benchmark_evidence:` 时，首个 `nextActions` 是 ``run `/round --json --run-benchmark --fail-on-command` ``，不再先展示 `deepcli quickstart --json`。
   - 目的：让产品评分报告也能直接指向本轮最该执行的修复命令，和 `round` 的失败 gate 优先语义保持一致。

11. benchmark baseline 对比入口
   - 结果：新增 `deepcli benchmark compare [--baseline path] [--json]`，输出稳定 `deepcli.benchmark.compare.v1`。
   - 命令只读取 `.deepcli/benchmarks/` artifact 和 workspace 内 baseline JSON，不执行 shell、不调用 Provider、不创建 session；baseline path 走 workspace path 校验并拒绝路径穿越。
   - 目的：让 benchmark 不只看本地历史趋势，还能和竞品、旧版本或人工维护 baseline 按 suite/case 对比状态和耗时差异，为 SOTA 产品循环提供横向证据。

12. benchmark baseline 模板入口
   - 结果：新增 `deepcli benchmark baseline-template [--name name] [--output path] [--json]`，输出可直接编辑的 `deepcli.benchmark.baseline.v1` JSON。
   - `--output` 写入 workspace 内 baseline 文件，默认覆盖 required benchmark preset，并留下待填写的 `status` 和 `durationMs`；生成后的文件可直接传给 `deepcli benchmark compare --baseline ...`。
   - 目的：把 baseline 对比从“知道隐藏 JSON 格式的人才能用”改成可发现、可复制、可闭环的本地工作流。

13. SOTA 产品循环 recipe
   - 结果：新增 `deepcli recipes sota --json`，并把 `product-loop`、`benchmark`、`round` 等 topic alias 归一到 `sota`。
   - `sota` recipe 串联 `scorecard`、`round`、`round --run-benchmark`、`benchmark status/trends`、`baseline-template`、`compare` 和 `benchmark gate`，全部作为本地只读命令清单输出，不创建 session、不调用 Provider。
   - 目的：让用户不必从长 README 或多个 nextActions 中拼产品循环路径，直接获得“检查缺口 -> 刷新本地证据 -> 横向 baseline 对比 -> gate”的可复制工作流。

14. SOTA recipe 接入失败报告
   - 结果：当 benchmark evidence 缺失时，`deepcli scorecard --json`、`deepcli round --json` 和 `deepcli benchmark status --json` 的 `nextActions` 都会暴露 `deepcli recipes sota --json`。
   - `scorecard` 和 `round` 仍保留原来的首要修复动作：`/round --json --run-benchmark --fail-on-command`；`benchmark status` 作为诊断入口会优先给出 SOTA recipe，帮助用户回到完整产品循环。
   - 目的：让上一轮新增的 SOTA recipe 不只是独立可发现入口，而是在用户看到失败 gate 时自然出现。

15. benchmark baseline 未填写引导
   - 结果：`deepcli benchmark compare --baseline ...` 在 baseline case 仍缺 `status` 或 `durationMs` 时，会保持 `incomplete` 并在 JSON/text `nextActions` 中提示先编辑对应 baseline 文件，再重新运行 compare。
   - 目的：让 `baseline-template -> 编辑 baseline -> compare` 的 SOTA 横向对比流程有明确卡点提示，避免用户按 recipe 生成模板后不知道为什么 compare 仍不完整。

16. scorecard 分类级 nextActions 修复优先
   - 结果：`deepcli scorecard --json` 不仅全局 `nextActions` 会把 gap remediation 放在前面，每个 category 自己的 `nextActions` 也会先展示本分类 gap 的修复动作。
   - 当前 benchmark evidence 缺失时，`benchmark_evidence.nextActions[0]` 是 ``run `/round --json --run-benchmark --fail-on-command` ``，不再先展示 `deepcli scorecard --json`。
   - 目的：让 TUI、外部 UI 或脚本按分类展示 scorecard 动作时，也能直接指向当前失败项的修复路径。

17. round scorecard 摘要保留分类级 nextActions
   - 结果：`deepcli round --json` 内嵌的 `scorecard.categories[]` 摘要现在会保留每个分类的 `nextActions`。
   - 当前 benchmark evidence 缺失时，round 报告里的 `scorecard.categories[] | select(.id=="benchmark_evidence") | .nextActions[0]` 也是 ``run `/round --json --run-benchmark --fail-on-command` ``。
   - 目的：让 TUI、外部 UI 或脚本只读取一份 `deepcli.round.v1` 报告，也能按分类展示修复动作，不必再额外调用 `deepcli scorecard --json`。

## 当前产品自评

最近自评中，`scorecard` 为 77/80，主要缺口是当前仓库没有保留本地 benchmark evidence artifact。这个缺口是有意保留的，因为 benchmark evidence 是本地忽略产物，不应推送到远程仓库。

本轮本地验收可通过 `deepcli round --json --run-benchmark --fail-on-command` 重新生成 benchmark evidence，使本地 `scorecard` 达到 80/80、`benchmark status` 为 ready；这些 `.deepcli/benchmarks/` artifact 仍然只作为本地证据，不进入 Git 提交。

下一轮产品设计应继续从真实使用阻力中选一个高价值缺口，而不是只为了让分数变绿而提交本地 artifact；本轮已补齐 baseline 模板未填写时的 compare 引导、scorecard 分类级 nextActions 排序，以及 round 摘要中的分类级 nextActions 透传，下一轮可继续关注 benchmark evidence 运行体验、TUI 可观测性或恢复历史的真实交互阻力。

## 常用检查命令

提交前建议至少运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
./scripts/deepcli help goal
./scripts/deepcli help plan
./scripts/deepcli help fork
./scripts/deepcli preflight --json
./scripts/deepcli review
```

产品循环检查：

```bash
./scripts/deepcli scorecard --json
./scripts/deepcli recipes sota --json
./scripts/deepcli round --json
./scripts/deepcli round --json --run-benchmark --fail-on-command
./scripts/deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
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
