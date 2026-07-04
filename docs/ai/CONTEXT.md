# deepcli 当前对话上下文

> 持续更新中：本文件用于把当前长期产品迭代对话的关键上下文落到仓库内，方便 deepcli、Codex 或其他 agent 在新会话中继续工作。

## 当前长期目标

持续执行“产品设计师检查缺口 -> 工程师实现功能 -> 验证 -> 再次产品检查”的循环，直到 deepcli 达到 SOTA local-first AI coding CLI 水平。该目标尚未完成，不应因为某一轮通过测试或完成提交就标记为结束。

## 当前产品收束决策

下一阶段优先级调整为“小而精”的核心能力路线，先停止围绕细枝末节的小命令继续扩张。后续实现顺序应先完成 harness 化的代码重构，用可重复场景约束核心 Agent 行为、命令契约、工具调用、会话状态和 UI 投影；等 harness 足够稳定后，再继续完善核心产品功能。

当前确认的重要功能范围：

- 会话系统：会话持久化、恢复、搜索、fork、状态投影、失败恢复和长任务续跑。
- 工具调用：统一工具声明、参数校验、权限接入、生命周期记录、失败回传和可测试 fake tool。
- 沙箱与权限：本地文件、shell、Git、网络和高风险操作的边界控制、审批和审计。
- 上下文压缩：从当前启发式压缩升级为可观察、可测试的 context manager，明确保留/丢弃原因。
- UI：聚焦核心任务体验，只保留对目标、计划、工具、diff、测试、审批、会话和上下文状态有直接帮助的视图。
- `/goal`：长期目标契约、停止条件、验收命令和 gate。
- `/fork`：复制已持久化上下文、恢复验证和并行探索。
- `/plan`：需求澄清、假设、验收标准和后续实现计划入口。
- `/harness`：核心重构优先事项，用离线 fake provider / fake tool / fixture workspace 验证 Agent 链路，而不是只作为普通小命令追加。
- LLM wiki：本地项目知识库，沉淀架构、约定、决策、关键文件和常用流程，后续由 context manager 按需注入。

在 harness 化重构完成前，不应优先实现新的小型 slash 命令、额外 benchmark 子命令、更多诊断变体或非核心 UI tab；除非它们直接服务上述核心能力或 harness 验收。

## 当前仓库状态基线

- 工作目录：当前 deepcli 仓库根目录
- 默认分支：`main`
- 远程仓库：`https://github.com/zero-kotori/deepcli.git`
- 目标提交身份：`zero-kotori <kotorizero8@gmail.com>`
- 产品命名：统一使用 `deepcli`
- 本地忽略产物：`.deepcli/benchmarks/`、`.deepcli/baselines/`、`.deepcli/exports/`、`.deepcli/support/`、credentials、logs、sessions 等不得提交

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
   - `/plan` 无参数保留查看执行计划；带需求文本时进入模型驱动的只读规划模式，模型会读代码上下文、必要时通过 `ask_user_question` 入队定制问题，并返回具体实现计划。
   - `/fork` 复制已持久化 session 上下文到新 session id，默认打开新 macOS Terminal 执行 `deepcli resume <new_id>`；Agent 运行中也可复制当前已落盘上下文，但当前运行中的后台 Agent 任务不热分叉。

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
   - 当唯一剩余缺口属于 `benchmark_evidence:` 时，首个 `nextActions` 是 `deepcli round --json --run-benchmark --fail-on-command`，不再先展示 `deepcli quickstart --json`。
   - 目的：让产品评分报告也能直接指向本轮最该执行的修复命令，和 `round` 的失败 gate 优先语义保持一致。

11. benchmark baseline 对比入口
   - 结果：新增 `deepcli benchmark compare [--baseline path] [--json]`，输出稳定 `deepcli.benchmark.compare.v1`。
   - 命令只读取 `.deepcli/benchmarks/` artifact 和 workspace 内 baseline JSON，不执行 shell、不调用 Provider、不创建 session；baseline path 走 workspace path 校验并拒绝路径穿越。
   - 目的：让 benchmark 不只看本地历史趋势，还能和竞品、旧版本或人工维护 baseline 按 suite/case 对比状态和耗时差异，为 SOTA 产品循环提供横向证据。

12. benchmark baseline 模板入口
   - 结果：新增 `deepcli benchmark baseline-template [--name name] [--output path] [--json]`，输出可直接编辑的 `deepcli.benchmark.baseline.v1` JSON。
   - `--output` 写入 workspace 内 baseline 文件，默认覆盖 required benchmark preset，并留下待填写的 case `status` 和 `durationMs`；生成后的文件可直接传给 `deepcli benchmark compare --baseline ...`。
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
   - 当前 benchmark evidence 缺失时，`benchmark_evidence.nextActions[0]` 是 `deepcli round --json --run-benchmark --fail-on-command`，不再先展示 `deepcli scorecard --json`。
   - 目的：让 TUI、外部 UI 或脚本按分类展示 scorecard 动作时，也能直接指向当前失败项的修复路径。

17. round scorecard 摘要保留分类级 nextActions
   - 结果：`deepcli round --json` 内嵌的 `scorecard.categories[]` 摘要现在会保留每个分类的 `nextActions`。
   - 当前 benchmark evidence 缺失时，round 报告里的 `scorecard.categories[] | select(.id=="benchmark_evidence") | .nextActions[0]` 也是 `deepcli round --json --run-benchmark --fail-on-command`。
   - 目的：让 TUI、外部 UI 或脚本只读取一份 `deepcli.round.v1` 报告，也能按分类展示修复动作，不必再额外调用 `deepcli scorecard --json`。

18. scorecard 全局 nextActions 按 gap 聚焦
   - 结果：`deepcli scorecard --json` 在存在 gaps 时，全局 `nextActions` 不再混入所有 strong category 的通用探索命令，而是聚焦 priority 修复动作、有 gap 分类动作和 SOTA 产品循环动作。
   - 当前 benchmark evidence 缺失时，`scorecard.nextActions[0]` 仍是 `deepcli round --json --run-benchmark --fail-on-command`，但 `deepcli quickstart --json` 这类 strong category 导航只保留在对应 category 的 `nextActions` 中。
   - 目的：让 scorecard 全局动作更像本轮修复队列，同时保留分类级完整导航，减少用户在高分但有单一缺口时被大量无关动作干扰。

19. scorecard nextActions 统一为可执行 CLI 命令
   - 结果：`deepcli scorecard --json` 和 round 内嵌 scorecard 摘要中的 benchmark 修复动作不再输出 ``run `/round ...` `` 这类说明性 slash 文本，而是统一输出 `deepcli round --json --run-benchmark --fail-on-command`。
   - 当前全局 `scorecard.nextActions` 和 `benchmark_evidence.nextActions` 均不包含以 ``run `/`` 开头的动作，脚本和用户可以直接复制执行。
   - 目的：让 one-shot JSON 的 nextActions 成为可执行命令清单，减少 TUI slash 命令、shell 命令和说明性文本混用带来的集成成本。

20. benchmark preset gap 修复提示统一为可执行 CLI 命令
   - 结果：`deepcli benchmark status --json` 和 `deepcli round --json` 内嵌的 `benchmarkStatus.presetCoverage.requiredStatus[].gap` 不再输出 ``run `/benchmark ...` `` 这类说明性 slash 文本，而是使用 `deepcli benchmark run-suite --json --fail-on-command`。
   - `deepcli benchmark show latest` 在没有本地 artifact 时，也会提示可直接执行的 `deepcli benchmark run-suite --json --fail-on-command`、`deepcli benchmark run --preset cargo-test --json --fail-on-command` 或 `deepcli benchmark record`。
   - 目的：让 benchmark evidence JSON 和错误提示中的修复路径与 scorecard/round nextActions 一致，降低外部 UI、脚本和用户复制执行的集成成本。

21. recipes nextActions 统一为可执行 CLI 命令
   - 结果：`deepcli recipes <topic> --json` 的 `nextActions` 不再输出 ``run `/...` `` 或带自然语言说明的 slash 文本，而是统一输出 `deepcli ...` 命令。
   - `deepcli recipes sota --json` 的顶层 `nextActions` 会根据当前 round 状态优先展示失败 gate 的直接修复动作，并保留 baseline compare 命令。
   - 目的：让 recipes 作为 TUI、外部 UI 和团队脚本的工作流目录时，`commands` 与 `nextActions` 都是可复制执行的命令清单，说明性上下文留给 `notes` 和 report。

22. scorecard nextActions 去除自引用跳转
   - 结果：当当前唯一 gap 属于 benchmark evidence 时，`deepcli scorecard --json` 的全局 `nextActions` 和 `benchmark_evidence.nextActions` 不再包含 `deepcli scorecard --json`。
   - 当前首个动作仍是 `deepcli round --json --run-benchmark --fail-on-command`，并保留 `deepcli recipes sota --json`、benchmark suite、gate、trends 和 preflight 等后续动作。
   - 目的：让 scorecard 报告输出的动作队列更像本轮修复队列，避免用户、TUI 或脚本刚读完 scorecard 又被引导回同一份报告。

23. round nextActions 去除自引用跳转
   - 结果：`deepcli round --json` 的外层 `nextActions` 不再包含 `deepcli round --json`。
   - 当前 scorecard gate 通过且只剩 benchmark evidence 缺口时，首个动作仍是 `deepcli round --json --run-benchmark --fail-on-command`，后续保留 `deepcli recipes sota --json`、benchmark suite、status、gate、trends、preflight 和 gate。
   - 目的：让 round 报告成为下一步执行队列，而不是让 TUI、外部 UI 或用户在同一个只读产品报告里循环。

24. benchmark status nextActions 隐藏空状态清理
   - 结果：当 `.deepcli/benchmarks/` 下没有 artifact 时，`deepcli benchmark status --json` 的 `nextActions` 不再包含 `deepcli benchmark clean --dry-run --json`。
   - 已有本地 artifact 的 weak、incomplete、failing、stale 或 ready 状态仍会展示 dry-run clean，作为证据维护动作。
   - 目的：让 missing 状态优先引导用户生成 benchmark evidence，而不是推荐一个不会产生价值的空目录清理步骤。

25. scorecard benchmark 修复队列去除 round 只读跳转
   - 结果：当当前唯一 gap 属于 benchmark evidence 时，`deepcli scorecard --json` 的全局 `nextActions` 和 `benchmark_evidence.nextActions` 都不会包含 `deepcli round --json`。
   - 首个动作仍是可执行修复命令 `deepcli round --json --run-benchmark --fail-on-command`，后续保留 recipes、benchmark suite、gate、trends、status 和 preflight 等动作。
   - 目的：让 scorecard 的修复队列只包含直接修复或有上下文增益的动作，避免用户在 scorecard 和只读 round 报告之间循环。

26. fork 上下文复制透明化
   - 结果：`deepcli fork --json` 增加 `contextCopy` 与 `nextActions`，明确源会话状态、复制模式、是否支持热分叉、运行中任务限制和恢复命令。
   - 文本输出同步展示 `source state`、`context copy`、warning 和 next actions；源会话处于 running 状态时会提示 `deepcli stop` 或等待任务结束后重新 fork。
   - 目的：让用户在“重新开一个一样的终端并继续上下文”场景下能准确理解 fork 复制的是已持久化会话文件，而不是正在运行的 Agent 内存状态。

27. benchmark trends 文本证据格式修复
   - 结果：`deepcli benchmark trends` 在单样本、无 previous duration delta 的情况下显示 `duration_delta=n/a`，不再错误显示 `duration_delta=n/ams`。
   - 目的：让 benchmark 证据报告的终端文本保持专业、可读，避免 SOTA 产品循环中的本地证据给出明显格式噪声。

28. scorecard ready 状态下一步动作聚焦
   - 结果：当 `deepcli scorecard --json` 没有 gaps 且状态为 ok 时，顶层 `nextActions` 会切换为持续验收动作：`deepcli round --json`、`deepcli preflight --json`、`deepcli gate --json`、`deepcli recipes sota --json`、`deepcli benchmark trends --json`、`deepcli benchmark status --json`，以及 `--from-current`、手工 baseline template 或 baseline compare。
   - 如果 benchmark evidence 已 ready 但 trends 仍是 `insufficient_history` 或 `regression`，顶层首项会改为 `deepcli round --json --run-benchmark --fail-on-command`，和当前 round gate 的修复路径保持一致。
   - 分类级 `categories[].nextActions` 仍保留对应分类的探索和诊断命令，TUI 或外部 UI 仍可展开查看，但顶层列表不再把 `quickstart`、`completion`、`status` 等所有强分类 discovery 命令混成一组。
   - 目的：ready 报告应该指导用户继续验收、观察趋势和做横向对比，而不是给出像命令大全一样的下一步列表。

29. benchmark trends 单样本历史不足状态
   - 结果：`deepcli benchmark trends --json` 在已有 artifact 但所有 case 都没有 previous 样本时返回 `status=insufficient_history`。
   - `nextActions` 会优先提示 `deepcli round --json --run-benchmark --fail-on-command`，文本报告同步显示 `status: insufficient_history`；低层 `deepcli benchmark run-suite --json --fail-on-command` 仍保留为后续动作。
   - 目的：让 scorecard ready 后推荐的趋势入口真实反映“还没有趋势可比样本”，避免单样本本地证据被误读为趋势已经充分。

30. round 聚合 benchmark trends gate
   - 结果：当 benchmark evidence 已 ready 但 benchmark trends 返回 `insufficient_history` 或 `regression` 时，`deepcli round --json` 会输出 `benchmark_trends` failed gate、`benchmark_trends:` gap 和优先修复动作。
   - 当前单样本趋势不足时，round 的首个 `nextActions` 是 `deepcli round --json --run-benchmark --fail-on-command`，而不是继续显示 ready 或只提示低层 benchmark 子命令。
   - 目的：让主产品循环直接暴露趋势证据质量，避免用户只看 `round.ready=true` 而忽略 benchmark trends 还没有可比历史。

31. round benchmark trends 修复动作闭环
   - 结果：`benchmark_trends` gate 的单样本历史不足修复动作使用 `deepcli round --json --run-benchmark --fail-on-command`。
   - 目的：让用户执行一个命令即可生成第二组本地 benchmark 样本并立即看到更新后的 `deepcli.round.v1`，不需要先跑 `benchmark run-suite` 再手动回到 `round`。

32. 顶层命令帮助旗标转发
   - 结果：`deepcli fork --help`、`deepcli sessions -h` 和 provider 前缀下的 `deepcli deepseek fork --help` 会在 wrapper 与 Rust 二进制本体中转成对应 `/help` 主题。
   - 特殊别名会归一化到真实主题：`sessions/history` -> `session`，`models/providers` -> `model`。
   - Rust 直连入口同步把 `goal` 和 `fork` 登记为已知顶层命令；`goal` 命令族由本地 handler 返回结果或 active session 错误，不会因带参数而误发给 provider。
   - 目的：让用户按常规 CLI 习惯探索顶层命令，不必记住只能使用 `deepcli help fork` 这类入口。

33. benchmark trends 历史不足闭环动作
   - 结果：`deepcli benchmark trends --json` 在 `insufficient_history` 时首个 `nextActions` 改为 `deepcli round --json --run-benchmark --fail-on-command`。
   - 目的：让用户从 trends 报告里执行一个命令即可补 benchmark 样本并立即看到新的 round gate，不需要先跑 `benchmark run-suite` 再手动回到 `round`。

34. SOTA recipe 状态感知 nextActions
   - 结果：`deepcli recipes sota --json` 复用当前 `round` 的状态感知 `nextActions`，再补充 baseline compare。
   - 目的：当当前 round 已知需要 benchmark 修复或 trend 补样本时，产品循环 recipe 不再先推荐只读 `deepcli round --json`，而是直接给出能推进闭环的修复命令。

35. scorecard ready 状态感知 trend 修复动作
   - 结果：`deepcli scorecard --json` 在自身无 gaps 但 benchmark trends 仍需处理时，顶层 `nextActions[0]` 会使用 `deepcli round --json --run-benchmark --fail-on-command`。
   - 目的：让用户从 scorecard、round 或 SOTA recipe 进入产品循环时，都能看到同一个当前失败 gate 的直接修复命令。

36. TUI 运行中产品循环观察命令
   - 结果：Agent 正在 TUI 中运行时，可直接执行 `/privacy`、`/recipes`、`/scorecard`、read-only `/round`、read-only `/benchmark` 报告子命令和 `/preflight --dry-run`，不依赖正在后台执行的 `AgentRuntime`。
   - 限制：`/round --run-benchmark`、`/benchmark run*|record|baseline-template|clean` 和完整 `/preflight` 会执行 shell、写入 benchmark 证据或维护 artifact，运行中会提示先等待当前任务结束或 `/stop`。
   - 目的：长任务中用户可以查看产品循环状态、benchmark evidence、隐私扫描和下一步动作，不必为了观察而中断 Agent。

37. TUI running-safe 标记收敛
   - 结果：命令帮助与 slash command palette 的 `running-safe` 标记只保留当前运行中 TUI handler 实际支持的命令，不再把 `/version`、`/quickstart`、`/health`、`/check`、`/docker`、`/compiler`、`/models`、`/providers`、`/accept`、`/gate`、`/verify`、`/handoff` 等本地 one-shot 或空闲期命令误标为运行中可执行。
   - 目的：用户在 Agent 运行中看到 `(run)` 标记时，执行路径应与界面承诺一致，避免命令面板先提示可运行、随后又被 running handler 拒绝。

38. TUI 运行中 fork 持久化上下文
   - 结果：Agent 正在 TUI 中运行时，可直接执行 `/fork --current` 复制当前 session 已持久化上下文，并默认打开新 macOS Terminal 恢复到副本；`--no-open --json` 路径可用于验收和脚本。
   - 限制：运行中的 provider turn、工具调用和未落盘输出不会被热分叉；JSON 继续通过 `contextCopy.hotForkSupported=false`、`runningAgentState=true` 和 warning 暴露边界。
   - 目的：长任务中用户可以把当前上下文分支到新终端并行探索，不必为了分支历史而中断主任务。

39. Terminal dry-run 可验收报告
   - 结果：`/terminal` 和 `deepcli terminal` 支持 `--dry-run|--no-open`、`--json` 与 `--output path`；dry-run 不创建进程，JSON 输出稳定 `deepcli.terminal.v1`，包含 workspace、platform、supported、command、opened、nextActions 和原始 report。
   - TUI：Agent 运行中可执行 `/terminal --dry-run --json`，通过当前 session 的本地 handler 返回报告，不依赖后台 `AgentRuntime`。
   - 目的：用户和外部 UI 可以脚本化验收“同目录终端”能力，不必真的打开 Terminal 或依赖肉眼观察。

40. Fork dry-run 预览
   - 结果：`/fork` 和 `deepcli fork` 支持 `--dry-run|--preview`，输出稳定 `deepcli.session.fork.v1`、`status=dry_run`、`dryRun=true`、`fork=null`、`plannedFork`、`terminal.wouldOpen`、`contextCopy` 和 next actions。
   - 行为：dry-run 只解析源会话并预览复制计划，不创建 session、不复制文件、不打开 Terminal；`--no-open` 保持真实创建 fork 但跳过 Terminal 的旧语义。
   - 目的：用户和外部 UI 可以先确认“会从哪个上下文分支、是否处于 running、是否热复制”再执行真实 fork，避免预览污染历史。

41. Fork resume 健康检查
   - 结果：`/fork` 和 `deepcli fork` 支持 `--verify`；真实 fork 后 JSON 会输出 `verification`，包含 `status`、`resumeReady`、workspace/provider/model 匹配状态、fork state、resume command，以及 message/tool/test/diff/backup 计数一致性。
   - 行为：`--verify` 只读取源和副本的持久化 session 文件，不调用 Provider、不实际启动 `deepcli resume`；`--dry-run --verify` 只说明 dry-run 不会创建 fork，因此不运行 resume 健康检查。
   - 目的：用户在“重新开一个一样的终端并使用同样上下文交互”之前，可以用 `deepcli fork --current --no-open --verify --json` 脚本化验收副本是否可恢复，而不是只依赖肉眼观察新 Terminal。

42. Resume dry-run 预览
   - 结果：`/resume` 和 `deepcli resume` 支持 `--dry-run|--preview`、`--json` 与 `--output path`，输出稳定 `deepcli.resume.preview.v1`，包含 selected session、activity、summary、recentMessages、resumeCommand 和 nextActions。
   - 行为：preview 只读取持久化 session 文件，不创建 session、不进入 TUI、不调用 Provider；无显式 id 时回退到最近有可恢复上下文的会话，显式 id 支持唯一短前缀，`--output` 限制在 workspace 内。
   - 目的：用户或外部 UI 可以在执行 `deepcli resume <id>` 之前确认将恢复的是哪段上下文，也能接在 fork verify 的 `resumeCommand` 后继续做非交互式验收。

43. Resume 候选去噪
   - 结果：`deepcli resume` picker、`/resume` 列表和 `resume --dry-run --json` 无显式 id 时使用同一套可恢复上下文判定，跳过只包含工具、测试或审计记录的诊断型 session。
   - 行为：候选需要有消息、summary、审批/旁路问题、计划或 goal；只有成功/失败工具、测试或审计记录的诊断型 session 不会进入无 id resume 候选，显式 session id 仍可预览或恢复指定 session，便于诊断。
   - 目的：避免最近的本地检查或工具-only one-shot 记录遮蔽真正的历史对话，让恢复入口默认指向用户能继续交互的上下文。

44. Preflight 运行诊断摘要
   - 结果：`/preflight` 和 `deepcli preflight --json` 会在文本报告中展示 diagnostics 行，并在 `deepcli.preflight.v1` JSON 中输出 `diagnostics` 对象。
   - 行为：诊断汇总 `totalDurationMs`、`measuredChecks`、`slowestCheck`、`largestOutputCheck` 和 `failedRequiredChecks`；dry-run 没有实测耗时时仍保留 JSON 字段，文本报告只在有可用诊断时展示。
   - 目的：发布前检查较慢或输出较长时，用户无需手动翻整份报告就能定位最慢检查、最大噪声来源和必须修复的 required check。

45. Benchmark status/summary JSON 内嵌 report
   - 结果：`deepcli benchmark status --json` 和 `deepcli benchmark summary --json` 现在都输出 `report` 字段，与 `trends`、`compare`、`scorecard` 等 JSON 报告保持一致。
   - 行为：`report` 复用对应文本 formatter，包含 workspace、状态、artifact/case 摘要、gaps 和 next actions，不额外读取或写入 benchmark artifact。
   - 目的：TUI、外部 UI 和脚本拿到 JSON 后即可展示人类可读摘要，不需要为了同一份信息再次运行非 JSON 命令。

46. Resume 低信息澄清会话去噪
   - 结果：无显式 id 的 `deepcli resume --dry-run --json`、resume picker 和 `/resume` 列表会跳过只包含低信息用户输入和 deepcli 本地澄清回复的会话，即使该澄清回复已经写入 summary。
   - 行为：显式 session id 仍可预览或恢复这类会话；带 goal、plan、审批或旁路问题的会话仍优先保留为可恢复候选。
   - 目的：避免用户误输入 `1`、`ok`、`.` 这类短输入后生成的澄清会话遮蔽真正可继续的历史任务。

47. Resume 当前 workspace 与短任务去噪
   - 结果：无显式 id 的 `deepcli resume --dry-run --json`、resume picker 和 `/resume` 列表只从当前 workspace metadata 匹配的会话中选择候选，并跳过短小已完成的单轮任务会话，即使该会话包含只读工具调用、completed plan 或简短 summary。
   - 行为：显式 session id 仍可预览或恢复旧 workspace metadata 的会话，以及默认隐藏的短任务会话；如果当前 workspace 没有可恢复对话，无 id dry-run 会明确报错而不是回退到旧路径。
   - 目的：避免项目改名、路径迁移或一次性问答记录遮蔽真正属于当前目录的历史对话。

48. Privacy 配置化禁用词扫描
   - 结果：`deepcli privacy` 支持 `privacy.blockedTerms` 和 `privacy.allowedTerms`；扫描 Git 提交元数据和历史文件内容时会把配置的禁用词作为 medium finding，allowed term 会进入 suppressed findings。
   - 行为：blocked term 样例统一显示为 `<blocked-term>`，不会在 JSON 或文本报告中再次泄漏原词；`--fail-on-findings` 会在未 suppressed 的 blocked term 命中时返回非零。
   - 目的：把旧产品名、公司邮箱、作者姓名、内部代号等项目特定发布风险纳入已有 preflight/privacy 本地门禁，避免靠人工 `rg` 记忆检查。

49. fork 默认候选去噪与 shell 误用提示
   - 结果：无 active session 且未传 session id 时，`deepcli fork` 使用当前 workspace 最近的可恢复对话上下文作为源会话，和 `resume` 一样跳过空会话、tool-only 或诊断型 session。
   - 行为：在 shell 中执行 `deepcli fork --current` 会明确提示省略 `--current`、传入 session id 或先用 `deepcli sessions --all --limit 20` 查看候选。
   - 目的：让“重新开一个一样的终端继续上下文”的默认路径更贴近用户意图，避免 fork 到没有可继续消息的诊断记录。

50. fork JSON 错误结构化
   - 结果：`deepcli fork --json` 在没有 active session 或没有可恢复源会话时，不再只输出纯文本错误，而是返回 `deepcli.session.fork.v1`、`status=error`、`error.code`、`error.message`、`nextActions` 和 `report`。
   - 行为：结构化错误会保持 `source=null`、`fork=null`、`plannedFork=null`、`contextCopy=null`，并在 `--output` 合法时先写入 JSON artifact 再非零退出。
   - 目的：让脚本、TUI 和外部历史 UI 能稳定处理 fork 的预期失败状态，不必为错误路径单独解析 stderr 文本。

51. benchmark 证据 freshness 可见性
   - 结果：`deepcli benchmark status --json` 和 `deepcli round --json` 内嵌 benchmark status 都输出 `freshness`，包含 `status`、`latestMeaningfulAge`、`latestMeaningfulAgeSeconds`、`refreshAfterDays`、`staleAfterDays`、`refreshRecommended` 和 `refreshAction`。
   - 行为：所有 required preset 证据都小于 1 天显示 `fresh`；任一 required preset 超过 1 天但未超过 7 天仍保持 `ready`，但标为 `aging` 并把 `deepcli round --json --run-benchmark --fail-on-command` 放到刷新动作前面；超过 stale 阈值继续按原有 `stale` gate 失败。
   - 目的：让用户和外部 UI 区分“本地证据仍有效”和“刚刚验证过”，避免 ready 报告隐藏证据年龄。

52. SOTA recipe baseline-aware nextActions
   - 结果：`deepcli recipes sota --json` 的顶层 `nextActions` 会检查默认 baseline 文件 `.deepcli/baselines/competitor.json`。
   - 行为：baseline 缺失且当前 artifact 可完整捕获时，先推荐 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再保留 `deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`；baseline 文件存在后才推荐 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`。recipe 的 commands 清单仍保留 template 和 compare 两个完整步骤。
   - 目的：避免产品循环入口在 ready 状态下给出一个因为默认 baseline 缺失而必然失败的 compare 命令，让用户按可执行顺序完成竞品或旧版本对比。

53. scorecard ready baseline-aware nextActions
   - 结果：`deepcli scorecard --json` 在没有 gaps 且状态为 ok 时，顶层 `nextActions` 也会检查默认 baseline 文件 `.deepcli/baselines/competitor.json`。
   - 行为：baseline 缺失且当前 artifact 可完整捕获时，先推荐 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再保留 `deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`；baseline 文件存在后才推荐 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`。如果 benchmark trends 需要补历史或处理回归，原有首项修复动作保持优先。
   - 目的：让 scorecard 和 SOTA recipe 两个产品循环入口都只推荐当前可执行的 baseline 下一步，避免同类入口给出互相矛盾的动作。

54. round ready baseline-aware nextActions
   - 结果：`deepcli round --json` 在所有 gates 通过且 round ready 时，外层 `nextActions` 会在 preflight/gate 后检查默认 baseline 文件 `.deepcli/baselines/competitor.json`。
   - 行为：baseline 缺失且当前 artifact 可完整捕获时，先推荐 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再保留 `deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json`；baseline 文件存在后才推荐 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`。benchmark evidence、trend 或 goal gate 未 ready 时，原有失败 gate 修复动作继续优先。
   - 目的：让主产品循环报告本身也不会在 ready 状态下漏掉横向 baseline 证据，避免用户只运行 `round --json` 时停在 preflight/gate，而看不到下一步 SOTA 对比动作。

55. baseline-template 自带后续动作
   - 结果：`deepcli benchmark baseline-template --json --output ...` 生成的 `deepcli.benchmark.baseline.v1` baseline 文件现在包含顶层 `status=needs_values`、`nextActions` 和 `report`。
   - 行为：模板仍保留每个 required case 的待填写 `status`/`durationMs`，可被 `benchmark compare` 直接读取；stdout 和写入文件都会提示先编辑对应 baseline 文件，再运行 `deepcli benchmark compare --baseline <path> --json`。
   - 目的：让用户执行 round/scorecard/recipes 推荐的 baseline-template 后不会停在一份裸 JSON 上，而是能继续完成 baseline 填写和对比。

56. baseline-template 捕获当前 benchmark
   - 结果：`deepcli benchmark baseline-template --from-current --json --output ...` 会从最新 required benchmark artifact 捕获每个 case 的 `status` 和 `durationMs`。
   - 行为：当 required cases 都有可用 artifact 和耗时时，baseline 顶层 `status=ready`，`nextActions` 直接指向 `deepcli benchmark compare --baseline <path> --json`；缺少 artifact 或耗时时仍保持 `needs_values`，并提示补跑 benchmark 或编辑 baseline。
   - 目的：让用户可以把当前版本、旧版本或手工跑完的对照版本保存成可立即 compare 的本地基线，不必手工填写每个 required case。

57. ready 产品循环优先推荐当前 baseline 捕获
   - 结果：`scorecard`、`round` 和 `recipes sota` 在默认 competitor baseline 缺失且本地 required benchmark artifact 可完整捕获时，会先推荐 `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`。
   - 行为：该动作排在手工 `.deepcli/baselines/competitor.json` 模板之前；如果默认 competitor baseline 已 ready，才推荐 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`；如果当前 artifact 缺失或缺少 duration，则仍只推荐手工 baseline template。
   - 目的：让 ready 产品循环直接暴露零手填的本地基线捕获路径，同时保留竞品或旧版本手工 baseline 工作流。

58. fork workspace-aware 恢复命令
   - 结果：真实 `deepcli fork --json` 会在 `terminal.workspaceResumeCommand` 中输出 shell-safe 的 `cd <workspace> && deepcli resume <new_id>`。
   - 行为：文本报告同步展示 `workspace resume command`；默认 macOS Terminal 打开逻辑和手动恢复命令复用同一生成函数，`--dry-run` 和源会话选择错误路径保持 `workspaceResumeCommand=null`，因为尚未创建 fork id。
   - 目的：用户在 `--no-open`、Terminal 打开失败或从其它目录手动恢复 fork 时，不会因为 `deepcli resume <id>` 查找的是当前目录下的 `.deepcli/sessions` 而误以为上下文丢失。

59. restore-backup 结构化预览
   - 结果：`/session restore-backup <name|latest> --dry-run --json` 输出稳定 `deepcli.session.restore_backup.v1`，包含 session metadata、backup metadata、target path、脱敏 diff、next actions 和 report。
   - 行为：`--output` 会把同一份 JSON 或文本写入 workspace 内路径；真实恢复也支持同一 schema 的 `--json`/`--output`，但仍通过 `write_file` 工具执行器写入并记录新的 backup/diff。dry-run 文本和 JSON 统一使用脱敏 diff。
   - 目的：让 TUI、外部恢复 UI 和脚本能在真正覆盖文件前结构化展示“会恢复哪个 backup 到哪个目标、会产生什么差异、下一步怎么执行”，同时避免预览 diff 泄漏备份或当前文件中的敏感值。

60. TUI 运行中 restore-backup 安全预览
   - 结果：Agent 运行中执行 `/session restore-backup latest --dry-run --json` 会通过同步预览路径输出 `deepcli.session.restore_backup.v1`，不需要等待后台任务结束。
   - 行为：运行中真实 `/session restore-backup latest` 仍会被拒绝；`/session restore-backup latest --dry-run --json --output ...` 也会被拒绝，因为它会写预览 artifact。shell one-shot 模式下的真实恢复和 dry-run `--output` 行为保持不变。
   - 目的：让用户在长任务运行时能安全查看备份恢复差异，同时避免后台 Agent 任务与用户恢复/写 artifact 并发修改 workspace。

61. TUI 运行中 `/session` read-only guard
   - 结果：Agent 运行中 `/session history --json` 等查看命令继续可用，但 `/session ... --output`、`/session rename`、`/session export` 和 `/session prune-empty --force` 会被统一拒绝。
   - 行为：被拒绝的命令不会写 `.deepcli/exports`，不会改当前 session title，也不会删除空 session；shell one-shot 模式下这些命令行为保持不变。
   - 目的：让 command palette 的 running-safe `/session` 标记真正代表“运行中可观察”，避免用户在后台 Agent 任务可能继续写文件时并发修改 session metadata、导出 artifact 或删除历史目录。

62. 环境 JSON nextActions 可执行化
   - 结果：`deepcli env check|plan|setup|test ... --json` 的顶层 `nextActions` 统一输出可直接复制到 shell 的 `deepcli ...` 命令，例如 `deepcli setup docker --smoke`、`deepcli env plan docker --smoke --json`、`deepcli accept --env-check docker --json`。
   - 行为：`commands` 字段和 report 正文继续保留 slash 命令，服务 TUI 和人工阅读；顶层 `nextActions` 不再输出 ``run `/...` `` 说明文本。
   - 目的：让环境自安装、环境预检和安装后验收能被脚本、外部 UI 和用户直接串起来，不需要再解析自然语言或手动把 slash 命令改成 shell 命令。

63. Tools 视图工具输出动作可见化
   - 结果：TUI 的 Tools 视图在工具调用折叠列表状态直接展示 `/session tools --limit 20 --current` 和 `/session tools --failed --limit 20 --current` 两个可编辑动作。
   - 行为：鼠标点击动作会预填 message box，不会直接执行长输出命令，也不会误展开工具项；展开某个工具详情时仍优先展示详情预览，并保留 `Ctrl-O`/`Ctrl-F` 预填完整或失败工具输出命令。
   - 目的：用户查看失败工具或完整工具记录时不再必须记快捷键或手输 `/session tools`，同时避免长任务中误触执行高噪声输出命令。

64. Quick actions run/edit 语义提示
   - 结果：TUI 任务观察面板的 quick actions 标题会根据动作类型显示 `Enter run`、`Enter edit` 或 `Enter run/edit`。
   - 行为：纯执行动作仍显示 `Enter run`；纯预填动作显示 `Enter edit`；同一面板同时包含执行和预填动作时显示 `Enter run/edit`。具体预填动作仍保留 `(edit)` 后缀。
   - 目的：避免用户看到 `/setup ... --smoke`、`/prompt render <name>` 或 Tools 输出动作时误以为 Enter 会直接执行，降低长任务和环境安装场景的误触成本。

65. Terminal workspaceCommand
   - 结果：`deepcli terminal --dry-run --json` 的 `deepcli.terminal.v1` 报告新增 `workspaceCommand`，文本 report 同步展示 `workspace command: cd <workspace>`。
   - 行为：未打开终端时，`nextActions[0]` 是 shell-safe 的 `cd <workspace>`；后续仍保留 `deepcli terminal` 和 `deepcli terminal --dry-run --json`。
   - 目的：当用户使用 `--no-open`、Terminal 打开失败、外部 UI 只做预览，或处在非 macOS 平台时，可以直接复制命令进入同一 workspace，不必从 JSON 的 workspace 字段手工拼命令。

66. Fork nextActions workspace-aware
   - 结果：真实 `deepcli fork --json` 的顶层 `nextActions[0]` 改为 `terminal.workspaceResumeCommand` 同款 shell-safe 命令：`cd <workspace> && deepcli resume <new_id>`。
   - 行为：短命令 `deepcli resume <new_id>` 仍作为后续 nextAction 保留；源会话 running 时继续追加 `deepcli stop` 和任务结束后重新 fork 的提示；Terminal 打开失败时文本报告也提示 workspace-aware 手动恢复命令。
   - 目的：外部 UI 或用户复制第一条 nextAction 时，无论当前 shell 在哪个目录，都能恢复 fork 副本，不会因为 `.deepcli/sessions` 查找当前目录而误判上下文丢失。

67. Resume 空候选 JSON 错误结构化
   - 结果：无显式 id 的 `deepcli resume --dry-run --json` 在当前 workspace 没有可恢复会话时，不再返回纯文本错误，而是保留 `deepcli.resume.preview.v1` schema 输出 `status=error`、`selected=null`、`error.code=no_resumable_context`、`error.message`、`nextActions` 和 `report`。
   - 行为：带 `--output` 时会先把同一份 JSON 写入 workspace 内目标文件，再通过 `CommandExit` 返回非零；非 JSON 路径继续保留原来的文本错误。
   - 目的：外部 UI、脚本和恢复选择器集成在空历史场景下也能稳定渲染恢复状态和下一步动作，不需要解析不可结构化的 stderr/stdout 文本。

68. Session search JSON nextActions
   - 结果：`deepcli session search <query> --json` 的 `deepcli.session.search.v1` 报告新增顶层 `nextActions`。
   - 行为：有命中时，动作围绕首个命中生成 `deepcli resume <short> --dry-run --json`、`deepcli session history <short> --limit 20`、`deepcli session next <short> --json` 和 `deepcli session diagnose <short> --json`；无命中时，动作指向 `deepcli sessions --all --limit 20`、`deepcli resume --dry-run --json` 和 `deepcli session list --json`。
   - 目的：恢复选择器、外部历史页和脚本拿到搜索结果后可以直接展示下一步，不需要自己拼接 session id 命令或解析终端文本。

69. Fork 无源错误 nextActions 候选发现
   - 结果：`deepcli fork --dry-run --json`、`deepcli fork --current --dry-run --json` 等源会话选择失败路径仍保留 `deepcli.session.fork.v1` 错误 schema，但顶层 `nextActions` 改为先给出非交互候选发现命令。
   - 行为：no-source 动作从 `deepcli resume` 调整为 `deepcli resume --dry-run --json`、`deepcli session list --all --limit 20 --json` 和 `deepcli sessions --all --limit 20`；非 JSON 错误提示也同步推荐结构化候选发现命令。
   - 目的：当当前 workspace 没有可恢复 deepcli session 时，外部 UI、脚本和用户不需要进入 TUI 或解析纯文本列表，也能继续发现候选或改用显式 session id fork。

70. Next JSON 动作可执行化
   - 结果：`deepcli next --json`、`deepcli session next --json` 和 `deepcli session diagnose --json` 不再从文本 report 反解析自然语言 slash bullet 作为 JSON 动作。
   - 行为：`deepcli.next.v1` 的 `nextActions`/`quickLinks` 以及 `deepcli.session.diagnose.v1` 的 `recommendedNextActions`/`quickLinks` 直接由 session state、审批、旁路问题、失败工具、失败测试和 plan 信号生成 `deepcli ...` 命令；文本 report 仍保留解释性 slash 语境。
   - 目的：TUI 面板、外部恢复 UI、脚本化 handoff 和下一 Agent 接力可以直接执行 JSON 动作，不需要从自然语言里抽取 `/resume`、`/approval list` 或 `/session tools`。

71. Benchmark aging 顶层刷新动作
   - 结果：当 benchmark evidence 仍为 ready 但 freshness 为 aging/stale 时，`scorecard --json`、`round --json` 和 `recipes sota --json` 的顶层 `nextActions[0]` 都会优先输出 `deepcli round --json --run-benchmark --fail-on-command`。
   - 行为：不改变 ready/gate 语义；`preflight`、`gate`、baseline template 和 baseline compare 仍保留在后续动作中。
   - 目的：用户从产品循环入口看到 aging 证据时可以先刷新本地 benchmark evidence，不需要展开 `benchmarkStatus.nextActions` 才发现推荐动作。

72. Quickstart/Selftest JSON 动作可执行化
   - 结果：`deepcli quickstart --json` 和 `deepcli selftest --json` 顶层 `nextActions` 不再输出说明性 slash-command prose，而是输出 `deepcli ...`、`cargo ...` 或 `git ...` 可直接执行命令。
   - 行为：`quickstart` 的 onboarding 解释仍在 `steps` 和 `report`；`selftest` 的诊断说明仍在 `report`，Git identity mismatch 会给出 `git config ...` 命令。
   - 目的：让首次启动、自检、外部 onboarding UI 和 CI 脚本不需要解析 `run \`/...\`` 文本即可继续下一步。

73. Cleanup JSON 动作可执行化
   - 结果：`deepcli cleanup sessions --json` 和 `deepcli session prune-empty --json` 顶层 `nextActions` 改为 `deepcli session prune-empty --force --json`、`deepcli session list ... --json`、`deepcli history ...` 这类 shell 可执行命令。
   - 行为：dry-run 仍只预览候选并跳过当前会话和有标题空会话；说明性清理摘要留在 `report`，不把 TUI slash 命令放入 JSON 顶层动作。
   - 目的：用户从 resume/fork 无源路径进入历史清理时，可以直接复制 JSON 动作执行维护命令，不需要把 `/session prune-empty --force` 手动改写成顶层 CLI 或离开 JSON 工作流。

74. Health/Version JSON 动作可执行化
   - 结果：`deepcli version/about/health/doctor --json` 顶层 `nextActions` 不再输出 `/quickstart` 或 `run \`/config validate\`` 这类说明性文本，而是输出 `deepcli quickstart`、`deepcli doctor --quick`、`deepcli config validate`、`deepcli setup docker --smoke` 等可执行命令。
   - 行为：health/doctor 的配置、凭据、环境说明仍在 report/providers/environment 字段；缺凭据时拆成 `credentials set/import-env/template` 三个命令；`doctor shell --json` 的 PATH、legacy command 和 completion 建议输出可复制的 shell 命令。
   - 目的：安装验收、支持排障 UI 和脚本可以直接渲染并执行修复动作，不需要解析 slash-command prose。

75. Inspect JSON 动作可执行化
   - 结果：`deepcli model show/list --json`、`deepcli timeout --json`、`deepcli logs --json`、`deepcli prompt list|get|render --json`、`deepcli skill list|run --json` 和 `deepcli agent list|show --json` 顶层 `nextActions` 不再输出 `/...` 或说明性 prose，而是输出可直接执行的 `deepcli ...` 命令。
   - 行为：有具体 prompt、skill 或 agent 任务时优先给出具体名称或短 id；空列表保留创建类命令模板；解释说明继续留在 `report`、条目字段或文本输出里。
   - 目的：TUI Library/Health/Logs/Agent 面板、外部设置页和脚本化验收可以直接消费 JSON 动作，不需要把 slash 命令或自然语言再改写成 shell 命令。

76. Running Fork JSON 动作可执行化
   - 结果：源会话处于 running/executing 状态时，真实 fork 和 dry-run fork 的 JSON 顶层 `nextActions` 不再输出“任务结束后再运行...”的说明性 prose，而是输出 `deepcli stop` 和 `deepcli fork --current` 这类可执行命令。
   - 行为：真实 fork 仍把 workspace-aware `cd <workspace> && deepcli resume <new_id>` 作为首个恢复动作；不热复制内存中 Agent 任务的限制继续保留在 `contextCopy.warning` 与 `report`。
   - 目的：外部 fork UI 和 TUI 运行中 fork 预览可以直接渲染动作按钮，不需要解析 running 分支里的自然语言。

77. Quick Preflight 隐私快路径
   - 结果：`preflight --dry-run --json` 的顶层 `nextActions` 改为可直接执行的 `deepcli preflight ... --json` 命令；`preflight --quick` 计划和执行的 privacy 检查改为 `deepcli privacy --json --fail-on-findings --no-history`。
   - 行为：full preflight 仍保留完整历史隐私扫描，作为提交、推送或发布前门禁；quick mode 只用于本地快速迭代，并继续跳过 clippy/gate。
   - 目的：减少本地产品循环中被完整 git 历史扫描拖慢的等待，同时不降低最终发布前隐私检查强度。

78. Status session actions 可执行化
   - 结果：`deepcli status --json` 的嵌套 `session.nextActions` 不再输出 `run \`/usage ...\``、`run \`/trace ...\``、`run \`/next ...\`` 或 `run \`/session diagnose ...\``，而是输出 `deepcli usage <id>`、`deepcli trace --limit 20 <id>`、`deepcli next <id>` 和 `deepcli session diagnose <id>`。
   - 行为：文本 status 报告仍保留面向 TUI 的 slash 命令说明；JSON 保持外部 UI 和脚本可直接消费的 shell 命令格式。
   - 目的：Status/Health/Result 面板可以直接把嵌套 session action 渲染成按钮，不需要解析自然语言或 slash-command prose。

79. Usage session actions 可执行化
   - 结果：`deepcli usage --json` 的嵌套 `session.nextActions` 不再输出 `run \`/trace --limit 20 ...\`` 或 `run \`/session diagnose ...\``，而是输出 `deepcli trace --limit 20 <id>` 和 `deepcli session diagnose <id>`。
   - 行为：文本 usage 报告仍保留面向 TUI 的诊断语境；JSON 中的动作保持 shell 可执行。
   - 目的：Usage 面板、支持页和脚本化慢响应诊断可以直接消费动作数组，不需要解析 slash-command prose。

80. Fork no-source actions 去占位符
   - 结果：`deepcli fork --dry-run --json` 和 `deepcli fork --current --dry-run --json` 在没有可恢复源会话时，顶层 `nextActions` 不再包含 `deepcli fork <session_id> --dry-run --json` 这类占位动作。
   - 行为：JSON 动作保留 `deepcli resume --dry-run --json`、`deepcli session list --all --limit 20 --json` 和 `deepcli sessions --all --limit 20` 作为可直接执行的候选发现入口；显式 session id 的说明留在错误 message/report 中。
   - 目的：外部 UI 和脚本可以把 fork no-source 的 nextActions 当作真实命令按钮渲染，不需要识别并过滤 `<session_id>` 占位符。

81. Inspect JSON actions 去占位符
   - 结果：`deepcli agent list --json`、`deepcli prompt list --json`、`deepcli skill list --json`、`deepcli model show/list --json` 和 `deepcli timeout --json` 的顶层 `nextActions` 不再输出 `deepcli agent spawn <task>`、`deepcli prompt render ... --file <path> key=value`、`deepcli prompt save <name> <body>`、`deepcli skill generate <name> <description>`、`deepcli model set <provider> [model]`、`deepcli model set <provider> <model>` 或 `deepcli timeout <seconds>` 这类模板动作。
   - 行为：没有具体对象可引用时，JSON 动作改为 `deepcli help ...` 或对应 list 命令；有具体 prompt、skill、agent task 或 provider 时仍优先输出具体 name/short id/provider 的可执行动作。
   - 目的：Library/Inspect/Settings 面板和脚本可以直接渲染 nextActions，不需要区分“动作按钮”和“需要编辑的命令模板”。

82. Test JSON actions 可执行化
   - 结果：`deepcli test discover --json` 和 `deepcli test run --json -- <command>` 的顶层 `nextActions` 不再输出 `run \`/test run\``、`run \`/accept --json\`` 或 `run \`/gate --json\`` 这类说明性 slash-command prose。
   - 行为：发现到测试命令时，JSON 动作输出 `deepcli test run --json` 和 shell-quoted 的 `deepcli test run --json -- <command>`；测试通过后输出 `deepcli accept --json`、`deepcli gate --json` 和同一条可执行 rerun 命令。
   - 目的：Tests/Deliver 面板、安装验收脚本和 CI glue 可以直接消费测试 JSON 动作，不需要把 TUI slash 文案改写成 shell 命令。

83. Fork current shell 误用动作直达
   - 结果：在普通 shell 中执行 `deepcli fork --current --dry-run --json` 且没有 active session 时，结构化错误的首个 `nextActions` 改为 `deepcli fork --dry-run --json`。
   - 行为：非 JSON 错误也输出同一份 fork error report 和 next action 列表；一般 no-source 场景仍保留 `deepcli resume --dry-run --json`、`deepcli session list --all --limit 20 --json` 和 `deepcli sessions --all --limit 20` 作为候选发现入口。
   - 目的：用户把 TUI-only 的 `--current` 带到 shell 时，可以直接退回“自动选择当前 workspace 最近可恢复上下文”的 fork 预览，而不是只看到候选列表并误以为 fork 功能不可用。

84. Diagnose/Support JSON actions 可执行化
   - 结果：`deepcli diagnose --json` 和 `deepcli support --json` 的顶层 `nextActions` 不再从文本 report 的 quick links 反解析，不再输出 `first-run guide: \`/quickstart\``、`/diagnose ...` slash prose 或 `<name>` 占位动作。
   - 行为：JSON 动作改为由 diagnose options 直接生成，例如 `deepcli quickstart`、`deepcli init --quick`、`deepcli diagnose --full-env --json`、`deepcli diagnose --probe-provider --json`、`deepcli model list --json`、`deepcli session diagnose --json` 和 `deepcli support .deepcli/support/latest --json`；support bundle 已生成时给出 `deepcli diagnose --json` 作为回到只读诊断的动作。
   - 目的：慢响应、凭据、环境或工具失败时，支持 UI 和脚本可以直接消费诊断 JSON 动作，不需要把 report quick links 从 slash 文案改写成 shell 命令。

85. Completion JSON actions 可执行化
   - 结果：`deepcli completion status <shell> --json` 和 `deepcli completion install <shell> --json` 的顶层 `nextActions` 不再输出 `install with ...`、`refresh with ...` 或 shell reload 说明文本。
   - 行为：缺失或过期时输出具体 shell 的 `deepcli completion install <shell> --force` 与 `deepcli completion status <shell> --json`；dry-run install 先给出 force install，再给出 status 与 `deepcli doctor shell --json`。
   - 目的：Shell 集成页、安装脚本和外部 UI 可以把 completion JSON 的 nextActions 直接渲染成命令按钮；重启或 reload shell 的解释保留在 report 中。

86. Support bundle manifest actions 可执行化
   - 结果：support bundle 内部 `manifest.json` 的顶层 `nextActions` 不再输出 `attach this support bundle ...`、`start from issue.md ...` 或带 `<dir>` 占位符的说明文本。
   - 行为：manifest 现在输出 `deepcli diagnose --json`、`deepcli support <bundle-dir> --json` 和 `deepcli diagnose --full-env --bundle <bundle-dir> --json`；人工支持说明保留在 `notes`。
   - 目的：支持页、外部 UI 或脚本读取 bundle manifest 时可以直接渲染动作按钮，不需要把支持说明和可执行命令混合解析。

87. Terminal opened actions 可执行化
   - 结果：`deepcli terminal --json` 在真实打开终端成功时，顶层 `nextActions` 不再输出 `use the opened terminal for parallel local work` 这类说明文本。
   - 行为：terminal JSON 在 dry-run、失败和 opened=true 状态下都输出可执行的 `cd <workspace>` 或 `deepcli ...` 命令；只有未打开时才保留 `deepcli terminal` 作为重试动作。
   - 目的：外部 UI 或脚本不需要为“终端已打开”成功状态单独过滤说明文本，仍可把 nextActions 当作命令按钮渲染。

88. Git read-only JSON 输出
   - 结果：`deepcli git status|diff|branch|message --json` 输出稳定 `deepcli.git.inspect.v1`，不再把 `--json` 当作被忽略的多余参数。
   - 行为：JSON 包含 kind、command、exitCode、stdout/stderr、raw、nextActions 和 report；`diff` 支持 `--staged|--cached`。read-only Git 子命令遇到未知 option 或多余参数会报错，不再静默返回空输出或纯文本输出。
   - 目的：外部 UI、脚本和发布检查可以可靠消费 Git 状态、diff、分支和提交信息建议，不会把空 stdout 加 exit 0 误判为结构化成功。

89. Git inspect output artifact
   - 结果：`deepcli git status|diff|branch|message [--json] --output <path>` 可把当前选择的文本或 JSON 只读输出写入 workspace-contained artifact。
   - 行为：`--output path` 与 `--output=path` 均复用统一路径校验，拒绝绝对路径、`..` 路径穿越和重复 output；写出的 artifact 使用与命令输出相同的 payload，CLI 打印时仍可按既有入口追加终端换行。
   - 目的：外部 UI、CI glue、发布检查和支持排障可以保存 Git inspection 证据，不必重跑命令或从终端输出复制 JSON。

90. TUI 运行中 read-only Git inspection
   - 结果：Agent 运行期间，TUI 可直接执行 `/git status|diff|branch|message [--json]` 来观察工作区和提交建议。
   - 行为：slash palette 和 `/help git` 将 `/git` 标为 running-safe；running handler 只允许不带 `--output` 的 read-only Git inspect，遇到 `/git ... --output`、`/git create-branch` 或 `/git commit` 会提示等待任务结束或先 `/stop`。
   - 目的：长任务期间用户不必停止 Agent 或离开 TUI，就能查看 Git 状态、diff、分支和 commit message 建议；同时避免运行中写 artifact 或执行 Git 写操作。

91. TUI 运行中旁路命令 artifact guard
   - 结果：Agent 运行期间，`/usage`、`/trace`、`/logs`、`/privacy`、`/fork`、`/recipes`、`/scorecard`、read-only `/round`、read-only `/benchmark`、`/selftest`、`/preflight --dry-run`、`/completion`、`/approval`、`/btw` 和 `/terminal` 继续可作为本地旁路命令执行，但带 `--output` 的 artifact 写入会被拒绝。
   - 行为：running handler 在进入各命令本体前统一检查 `--output`、`-o` 和 `--output=...`，提示等待当前任务结束或先 `/stop`，并保证不会创建 `.deepcli/exports/...` 文件。
   - 目的：长任务期间用户仍能观察状态、处理审批和旁路问题，但不会在后台 Agent 运行时写入导出 artifact，和 `/git`、`/session` 的运行中写入边界保持一致。

92. TUI 运行中 completion force install guard
   - 结果：Agent 运行期间，`/completion` 的 guide、status、JSON 和 install dry-run 仍可运行；`/completion install ... --force` 会被拒绝。
   - 行为：running handler 在 completion 本体前检查 `--force`，提示等待任务结束或先 `/stop`，避免写入用户 HOME 下的 shell completion 文件；`--output` 仍由旁路命令 artifact guard 拦截。
   - 目的：长任务期间用户可以继续查看和预览 shell completion 配置，但不会在后台 Agent 运行时误改本机 shell completion 安装状态。

93. Verify/Handoff JSON actions 可执行化
   - 结果：`deepcli verify --json`、`deepcli gate --json` 和 `deepcli handoff --json` 的顶层 `nextActions` 不再输出说明性 prose、反引号 slash 命令或 `<message>` 占位动作。
   - 行为：文本 report 仍保留适合 TUI 和人工阅读的 slash 命令建议；JSON 层从 report 的反引号命令中提取动作，统一归一为可直接执行的 `deepcli ...`、`cargo ...` 或 `git ...` 命令，并去重。
   - 目的：`round` ready 状态推荐 `deepcli gate --json` 后，外部 UI、CI glue 和脚本可以继续把 gate/handoff 的 nextActions 当作命令按钮消费，不需要解析自然语言或手动改写 slash 命令。

94. Fork/Terminal 可选终端 app
   - 结果：`/terminal` 与 `/fork` 支持 `--app <name>` / `--terminal-app <name>`，JSON/report 会展示终端 app；`terminal --app iTerm2 --dry-run --json` 的 nextActions 会保留 app 参数。
   - 行为：`/terminal` 使用 shell-quoted `open -a <app> .` 打开当前 workspace；`/fork` 默认仍用 Terminal，显式 iTerm2 时可自动执行 `deepcli resume <new_id>`，其他 app 不宣称支持自动 resume，只通过错误说明和 `workspaceResumeCommand` 引导手动恢复。
   - 目的：解决“重新开一个一样的终端”体验中硬编码 Apple Terminal 的阻力，让 iTerm2 等用户能明确选择自己的终端，同时避免对无法可靠脚本化的终端做虚假承诺。

95. Fork/Terminal 终端 app 默认偏好
   - 结果：`/terminal` 与 `/fork` 会读取 `DEEPCLI_TERMINAL_APP` 作为默认终端 app；显式 `--app` / `--terminal-app` 仍然优先。
   - 行为：未传 `--app` 时，`DEEPCLI_TERMINAL_APP=iTerm2 deepcli terminal --dry-run --json` 和对应 fork dry-run 会输出 iTerm2 app、命令和保留 app 的 nextActions；环境变量为空、含控制字符或非 UTF-8 时返回本地错误。
   - 目的：减少 iTerm2 等用户每次手输 `--app` 的成本，让“重新开一个一样的终端”更接近日常使用习惯，同时保持默认 Terminal 兼容。

96. Wrapper terminal 帮助可发现性
   - 结果：`deepcli --help` 的常用命令组和 DeepSeek/Kimi provider 快捷命令组都显式列出 `terminal`。
   - 行为：不改变已有路由；`deepcli terminal ...` 和 `deepcli deepseek terminal ...` 继续转发到本地 `/terminal`，帮助文本只负责让可用能力在首次查看 Usage 时可见。
   - 目的：上一轮已经补齐同目录终端能力，但顶层帮助仍只能从示例或专题 help 间接发现；本轮把可发现性与实际命令面收敛，减少用户记忆成本。

97. Wrapper Git inspect 帮助可发现性
   - 结果：`deepcli --help` 增加 `deepcli git status|diff|branch|message [--json] [--output path]` Usage 行，并在 DeepSeek/Kimi provider 快捷命令组显式列出 `git`。
   - 行为：不改变已有 Git inspect 路由；`deepcli git status --json` 和 `deepcli deepseek git status --json` 继续输出稳定 `deepcli.git.inspect.v1`。
   - 目的：只读 Git 状态、diff、分支和 commit message 建议已经是交付前高频能力，但顶层帮助缺少可发现入口；本轮让用户在首次查看 Usage 时就能看到 Git inspection 工作流。

98. Wrapper 协作队列帮助可发现性
   - 结果：`deepcli --help` 增加 `deepcli approval list [--json] [--output path]` 与 `deepcli btw ask <question>|list [--json] [--output path]` Usage 行，并在 DeepSeek/Kimi provider 快捷命令组显式列出 `approval` 和 `btw`。
   - 行为：不改变已有审批和 by-the-way 队列路由；`deepcli approval list --json` 与 `deepcli btw list --json` 继续输出稳定 schema。
   - 目的：审批请求和旁路问题是长任务协作中的关键队列，但此前顶层帮助只在 Examples 里露出；本轮让用户在 Usage 区直接发现运行中协作入口。

99. 协作队列 JSON actions 可执行化
   - 结果：`deepcli approval list --json` 和 `deepcli btw list --json` 现在都会输出顶层 `nextActions`。
   - 行为：approval 存在 pending 请求时优先给出具体 `deepcli approval approve <short_id>` 与 `deepcli approval deny <short_id>` 命令，并保留 session-scoped `--all --json` 和帮助入口；空审批队列不输出 approve/deny。btw list 不输出需要用户替换回答文本的占位 answer 命令，只给出 session-scoped `--json`、`--all --json` 和帮助入口。
   - 目的：让 TUI、外部 UI 和脚本在读取协作队列 JSON 时无需解析 report，也不会在空队列或开放问题上拿到不可执行的占位动作。

100. Wrapper 协作队列处理命令可发现性
   - 结果：`deepcli --help` 的 Usage 区新增 approval approve/deny/clear 与 btw answer/clear，Examples 区也加入对应可复制样例。
   - 行为：不改变既有路由；`deepcli approval approve <id>`、`deepcli approval deny <id>`、`deepcli approval clear --current`、`deepcli btw answer <id> ...` 和 `deepcli btw clear --current` 仍按原有 wrapper 规则转成对应 slash command。
   - 目的：上轮让 JSON 消费方可以拿到处理动作，本轮让人工用户从顶层帮助就能发现队列处理闭环，不必先知道 `/help approval` 或 `/help btw`。

101. 协作队列处理结果结构化
   - 结果：`approval approve|deny|clear --json` 输出 `deepcli.approval.action.v1`，`btw answer|clear --json` 输出 `deepcli.btw.action.v1`。
   - 行为：action JSON 包含 workspace、action、session、处理后的 approval/question 或 clearedCount、可执行 `nextActions` 和 report；`--output` 继续限制在 workspace 内 artifact。默认文本输出不变，wrapper 会原样转发 action JSON 参数。
   - 目的：让外部 UI 和脚本从 list JSON 拿到处理命令后，执行处理命令也能获得稳定结构化结果并立即刷新队列，而不是解析纯文本。

102. Git 写操作 dry-run 预览
   - 结果：`deepcli git create-branch <name> --dry-run --json` 和 `deepcli git commit <message> --dry-run --json` 输出稳定 `deepcli.git.action.v1`。
   - 行为：dry-run JSON 包含 workspace、action、subject、planned command、可执行 `nextActions` 和 report，并支持 workspace-contained `--output`；预览不会创建分支或提交。未知参数仍会在真实写操作前报错，真实 `create-branch` 和 `commit` 仍走权限策略。
   - 目的：上一轮只避免了 `--dry-run` 被静默吞掉造成误写；本轮补上用户自然期待的安全预览，让 shell 用户、TUI 和外部 UI 可以先确认 Git 写操作再执行。

103. Fork/Terminal 当前终端自动识别
   - 结果：`/terminal` 与 `/fork` 在没有显式 `--app` 且没有 `DEEPCLI_TERMINAL_APP` 时，会从 `TERM_PROGRAM` 推断已支持终端；`TERM_PROGRAM=iTerm.app` 自动选择 iTerm2，`TERM_PROGRAM=Apple_Terminal` 选择 Terminal。
   - 行为：终端 app 优先级为显式 `--app`/`--terminal-app`、`DEEPCLI_TERMINAL_APP`、`TERM_PROGRAM` 推断、Terminal；未知 `TERM_PROGRAM` 继续回退 Terminal，不宣称不支持的终端可自动 resume。dry-run JSON 与 nextActions 会保留推断出的 `--app iTerm2`，保证预览和真实执行一致。
   - 目的：用户问“重新开一个一样的终端”时，不应要求 iTerm 用户每次手动传 `--app iTerm2` 或预先设置环境变量；自动识别让 fork/terminal 更贴近日常终端使用习惯，同时保持 Terminal/iTerm2 自动 resume 边界清晰。

104. Recipes 顶层工作流清单
   - 结果：`deepcli recipes <topic> --json` 的顶层 JSON 增加 `title`、`summary` 和 `checklist[]`。
   - 行为：单 topic 时 title/summary 来自选中的 recipe，checklist 从命令链派生 `step`、`label` 和可执行 `command`；`recipes sota --json` 会直接给出 “Open SOTA product loop recipe”、“Inspect product gaps”、“Refresh benchmark evidence”等步骤，外部 UI 不需要解析 report 或嵌套 recipes。
   - 目的：`recipes sota` 是产品循环入口，但只有嵌套 recipe 和长文本 report 时，TUI/外部 UI 很难直接渲染成操作清单；顶层 checklist 让工作流目录更像可执行 playbook。

105. Scorecard 分类级工作流清单
   - 结果：`deepcli scorecard --json` 的每个 `categories[]` 增加 `checklist[]`，`deepcli.round.v1` 内嵌的 `scorecard.categories[]` 摘要也保留同一字段。
   - 行为：checklist 从分类级 `nextActions` 中可直接执行的 `deepcli ...` 动作派生 `step`、`label` 和 `command`，会过滤说明性 gap 文本和占位符；例如 Command Discovery 会给出 “Open quickstart readiness” -> `deepcli quickstart --json`，Benchmark Evidence 会给出 “Refresh benchmark evidence” -> `deepcli round --json --run-benchmark --fail-on-command`。
   - 目的：scorecard 已能给出分类级 nextActions，但外部 UI 仍要解析自然语言或自行命名按钮；分类级 checklist 让 scorecard/round 报告直接支撑可折叠修复面板和工作流按钮。

106. Round Gate 工作流清单
   - 结果：`deepcli round --json` 的每个 `gates[]` 增加 `checklist[]`。
   - 行为：有 `nextAction` 的 gate 会输出单步 checklist，包含 `step`、`label` 和可执行 `command`；无 nextAction 的 gate 输出空 checklist。Benchmark Evidence ready 时会给出 “Review benchmark summary” -> `deepcli benchmark summary --json`，失败时会给出 “Refresh benchmark evidence” -> `deepcli round --json --run-benchmark --fail-on-command`。
   - 目的：round 是每轮产品循环入口，但 gate 只有 `nextAction` 时，外部 UI 仍要自行命名和解释 gate 修复按钮；gate-level checklist 让 round 报告本身可以直接驱动门禁面板。

107. SOTA Recipe checklist baseline-aware
   - 结果：`deepcli recipes sota --json` 的顶层 `checklist[]` 会按当前 baseline 状态选择步骤。
   - 行为：默认 `.deepcli/baselines/competitor.json` 缺失且当前 benchmark artifact 可完整捕获时，checklist 展示 “Capture current benchmark baseline” -> `deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json`，再展示手工 competitor template；默认 baseline ready 时才展示 baseline compare；默认 baseline 存在但缺值或无效时先展示 `deepcli benchmark baselines --json`。静态 `recipes[].commands` 仍保留完整参考命令链。
   - 目的：外部 UI 渲染 SOTA recipe checklist 时不再把用户带到当前必然失败的 compare 步骤，和状态感知 `nextActions` 保持一致。

108. Scorecard 顶层工作流清单
   - 结果：`deepcli scorecard --json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：顶层 checklist 从全局 `nextActions` 中可执行的 `deepcli ...` 命令派生 `step`、`label` 和 `command`；ready 状态下会按持续验收队列展示 “Refresh benchmark evidence”、“Open SOTA product loop recipe”、“Capture current benchmark baseline”等动作。`categories[].checklist[]` 继续保留分类级动作。
   - 目的：scorecard 是产品循环入口之一，但此前只有全局 nextActions 和分类级 checklist，外部 UI 仍要自行给全局队列命名；顶层 checklist 让 scorecard 可直接驱动本轮全局修复/验收按钮。

109. Round 顶层工作流清单
   - 结果：`deepcli round --json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：顶层 checklist 从全局 `nextActions` 中可执行的 `deepcli ...` 命令派生 `step`、`label` 和 `command`；缺 benchmark evidence 时会包含 “Refresh benchmark evidence” -> `deepcli round --json --run-benchmark --fail-on-command`，ready 状态下会展示 preflight/gate 和 baseline 捕获或对比动作。`gates[].checklist[]` 和内嵌 `scorecard.categories[].checklist[]` 继续保留 gate 级与分类级动作。
   - 目的：round 是每轮产品循环的主入口，但此前只有全局 nextActions、gate checklist 和分类 checklist，外部 UI 仍要自行给 round 全局队列命名；顶层 checklist 让单份 round 报告就能驱动全局、gate 和分类三级操作面板。

110. Preflight 顶层发布检查清单
   - 结果：`deepcli preflight --json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：preflight checklist 从实际检查队列派生 `step`、`name`、`label`、`command`、`status` 和 `required`；dry-run 时状态为 planned，真实执行时随各检查结果更新。
   - 目的：round ready 后的第一步就是 preflight，此前外部 UI 仍要解析 `checks[]` 或 report 文本才能渲染发布检查队列；顶层 checklist 让产品循环从 round 到 preflight 的结构化操作面保持一致。

111. Verify/Gate/Handoff 顶层交付动作清单
   - 结果：`deepcli verify --json`、`deepcli gate --json` 和 `deepcli handoff --json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：交付 checklist 从 JSON 顶层 `nextActions` 派生 `step`、`label` 和 `command`，覆盖测试证据、环境验收、diff/review、session diffs 和 handoff 报告等高频动作。
   - 目的：round/preflight 后的交付链路此前仍只给可执行命令而没有可渲染清单；顶层 checklist 让外部 UI、TUI 面板和脚本可以连续展示产品循环、发布检查、验收 gate 和交付报告动作队列。

112. Benchmark 证据报告动作清单
   - 结果：`deepcli benchmark status --json`、`deepcli benchmark summary --json`、`deepcli benchmark trends --json` 和 `deepcli benchmark compare --json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：benchmark checklist 从可执行 `deepcli ...` nextActions 派生 `step`、`label` 和 `command`；compare 中的 `edit status and durationMs ...` 人工编辑提示继续保留在 `nextActions` 和 `report`，但不会进入 checklist。
   - 目的：scorecard/round/recipes 已能结构化渲染产品循环动作，但用户进入 benchmark 证据子报告后仍要自行解析 nextActions；顶层 checklist 让 SOTA 证据链中的 status、summary、trends 和 baseline compare 也能直接渲染动作队列。

113. Quickstart/Selftest 顶层动作清单
   - 结果：`deepcli quickstart --json` 和 `deepcli selftest --json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：checklist 从可执行 `deepcli ...`、`cargo ...` 或 `git ...` nextActions 派生 `step`、`label` 和 `command`；首次引导和诊断说明继续保留在 `steps`、`report` 等解释性字段。
   - 目的：新用户 onboarding 和产品自检入口此前只有可执行 nextActions，TUI、外部 onboarding UI、安装脚本或验收脚本仍要自行命名按钮；顶层 checklist 让启动链路和后续 scorecard/round/preflight/benchmark 动作面保持一致。

114. Health/Support 顶层动作清单
   - 结果：`deepcli version/about --json`、`deepcli doctor/health --json`、`deepcli diagnose/support --json` 和 support bundle `manifest.json` 的顶层 JSON 增加 `checklist[]`。
   - 行为：checklist 从可执行 `deepcli ...`、`cargo ...`、`git ...` 和 doctor shell 安装动作派生 `step`、`label` 和 `command`；配置、凭据、环境、shell 和人工支持说明继续保留在 `report`、`environment`、`shell` 或 `notes` 中。
   - 目的：健康检查、版本支持元数据和支持包此前已有可执行 nextActions，但 TUI、外部健康面板或支持面板仍要自行解析和命名动作；顶层 checklist 让支持诊断链路和 onboarding、scorecard、round、preflight、交付、benchmark 报告保持一致。

115. 观测/恢复报告动作清单
   - 结果：`deepcli status --json`、`deepcli usage --json`、`deepcli logs --json`、`deepcli next --json`、`deepcli session next --json` 和 `deepcli session diagnose --json` 增加可直接渲染的 `checklist[]`；`status/usage` 同时提供顶层 `checklist[]` 和 `session.checklist[]`。
   - 行为：checklist 从对应的 `session.nextActions`、顶层 `nextActions` 或 `recommendedNextActions` 中可执行动作派生 `step`、`label` 和 `command`；解释性原因继续保留在 `report`、`signals`、`diagnostics`、`quickLinks` 等字段。
   - 目的：观测面板、恢复面板和外部 UI 不再需要解析 action 字符串或自行给按钮命名，可以直接渲染可执行操作队列。

116. 恢复报告辅助链接动作清单
   - 结果：`deepcli next --json`、`deepcli session next --json` 和 `deepcli session diagnose --json` 增加 `quickLinkChecklist[]`。
   - 行为：`quickLinkChecklist[]` 从现有 `quickLinks` 派生 `step`、`label` 和 `command`，保留原 `quickLinks` 字符串数组兼容脚本；主恢复动作继续使用 `checklist[]`。
   - 目的：恢复 UI 可以把“要处理的主动作”和“可辅助跳转的链接”分区渲染，不再为 quick links 自行解析命令或命名按钮。

117. Session Search 历史恢复动作清单
   - 结果：`deepcli session search --json` 增加顶层 `checklist[]`。
   - 行为：`checklist[]` 从 `nextActions` 派生 `step`、`label` 和 `command`；命中场景覆盖 resume preview、history、next 和 diagnose，无命中场景覆盖 session list 与 resume preview。
   - 目的：恢复历史搜索此前只能给外部 UI 一组可执行命令，按钮命名和步骤编号还要各端自行解析；顶层 checklist 让历史搜索结果可以直接渲染恢复动作队列。

118. Resume Preview 恢复入口动作清单
   - 结果：`deepcli resume --dry-run --json` 和 `deepcli resume <id> --dry-run --json` 增加顶层 `checklist[]`。
   - 行为：preview 成功场景从 resume、session next 和 session diagnose 动作派生 checklist；无可恢复候选的 error 场景从 sessions、session list JSON 和 history 动作派生 checklist，并保留原 `nextActions` 兼容脚本。
   - 目的：恢复入口此前虽然已经结构化输出候选预览和错误动作，但外部恢复 UI 仍要自行解析 `nextActions` 命名按钮；顶层 checklist 让从 resume preview 到 session search、session next、session diagnose 的恢复链路保持一致。

119. Model Inspect 配置动作清单
   - 结果：`deepcli model show --json` 和 `deepcli model list --json` 增加顶层 `checklist[]`。
   - 行为：show 从模型列表和模型帮助动作派生 checklist；list 从具体 `deepcli model set ...` 切换动作派生 checklist，不输出 provider/model 占位命令。
   - 目的：TUI Health/模型页和外部设置 UI 可以直接渲染查看、切换和帮助动作，不需要自行解析 `nextActions` 或给模型按钮命名。

120. Fork JSON 恢复动作清单
   - 结果：真实 `deepcli fork --json`、`deepcli fork --dry-run --json` 和源会话选择失败的 `deepcli.session.fork.v1` error JSON 增加顶层 `checklist[]`。
   - 行为：真实 fork 从 workspace-aware resume、短 resume、stop 和 fork-current 动作派生 checklist；dry-run 从真实 fork 预览动作派生 checklist；no-source error 从 fork preview、resume preview 和 session list 动作派生 checklist。
   - 目的：外部 fork UI、TUI 运行中 fork 面板和脚本验收可以直接渲染“恢复副本、预览 fork、停止运行中任务、发现候选”等动作，不需要解析 `nextActions` 或为 shell 命令自行命名。

## 当前产品自评

当前本地自评中，`scorecard` 为 80/80，`benchmark status` 为 ready；如果本地 `.deepcli/benchmarks/` 只有每个 required case 的单条样本，`benchmark trends` 会返回 `insufficient_history`，`round` 会据此进入 `needs_attention` 并提示 `deepcli round --json --run-benchmark --fail-on-command`。当 benchmark evidence、trends 和 goal gates 都 ready 时，`round.nextActions` 会继续在 preflight/gate 后提示 `--from-current`、手工 baseline template 或 baseline compare；如果默认 competitor baseline 缺失且本地 artifact 可完整捕获，会先提示 `baseline-template --from-current` 生成 `status=ready` 的 compare-ready baseline，再提示手工 competitor baseline template。该结果依赖 `.deepcli/benchmarks/` 下的本地忽略证据 artifact，这些文件不应推送到远程仓库。

如果 fresh checkout 或清理后缺少本地 benchmark evidence，可通过 `deepcli round --json --run-benchmark --fail-on-command` 重新生成，使本地 `scorecard` 达到 80/80、`benchmark status` 为 ready；这些 `.deepcli/benchmarks/` artifact 仍然只作为本地证据，不进入 Git 提交。

下一轮产品设计应继续从真实使用阻力中选一个高价值缺口，而不是只为了让分数变绿而提交本地 artifact；本轮已补齐 baseline 模板未填写时的 compare 引导、baseline-template 自带后续动作、baseline-template 捕获当前 benchmark、ready 产品循环优先推荐当前 baseline 捕获、Fork/Terminal 可选终端 app、Fork/Terminal 终端 app 默认偏好、Fork/Terminal 当前终端自动识别、Recipes 顶层工作流清单、Scorecard 分类级工作流清单、Round Gate 工作流清单、SOTA Recipe checklist baseline-aware、Scorecard 顶层工作流清单、Round 顶层工作流清单、Preflight 顶层发布检查清单、Verify/Gate/Handoff 顶层交付动作清单、Benchmark 证据报告动作清单、Model Inspect 顶层动作清单、Fork JSON 恢复动作清单、Wrapper terminal 帮助可发现性、Wrapper Git inspect 帮助可发现性、Wrapper 协作队列帮助可发现性、fork workspace-aware 恢复命令、Fork nextActions workspace-aware、Resume 空候选 JSON 错误结构化、Resume Preview 顶层动作清单、Session search JSON nextActions、Session search 顶层动作清单、Fork current shell 误用动作直达、Diagnose/Support JSON actions 可执行化、Completion JSON actions 可执行化、Support bundle manifest actions 可执行化、Terminal opened actions 可执行化、Git read-only JSON 输出、Git inspect output artifact、TUI 运行中 read-only Git inspection、TUI 运行中旁路命令 artifact guard、TUI 运行中 completion force install guard、Verify/Handoff JSON actions 可执行化、Fork 无源错误 nextActions 候选发现、Fork no-source actions 去占位符、Inspect JSON actions 去占位符、Test JSON actions 可执行化、Next JSON 动作可执行化、Benchmark aging 顶层刷新动作、Quickstart/Selftest JSON 动作可执行化、Cleanup JSON 动作可执行化、Health/Version JSON 动作可执行化、Inspect JSON 动作可执行化、Running Fork JSON 动作可执行化、Quick Preflight 隐私快路径、Status session actions 可执行化、Usage session actions 可执行化、restore-backup 结构化预览、TUI 运行中 restore-backup 安全预览、TUI 运行中 `/session` read-only guard、环境 JSON nextActions 可执行化、Tools 视图工具输出动作可见化、Quick actions run/edit 语义提示、Terminal workspaceCommand、scorecard 分类级 nextActions 排序、round 摘要中的分类级 nextActions 透传、scorecard 全局 nextActions 的 gap-aware 聚焦、scorecard nextActions 的可执行 CLI 命令格式、benchmark preset gap 修复提示的可执行 CLI 命令格式、recipes nextActions 的可执行 CLI 命令格式、scorecard nextActions 的自引用跳转清理、round nextActions 的自引用跳转清理、benchmark status 空证据状态的 clean action 隐藏、scorecard benchmark 修复队列的 round 只读跳转回归测试、fork 上下文复制透明化、benchmark trends 文本证据格式修复、scorecard ready 状态下的下一步动作聚焦、benchmark trends 单样本历史不足状态、round 聚合 benchmark trends gate、round benchmark trends 修复动作闭环、顶层命令帮助旗标转发、benchmark trends 历史不足闭环动作、SOTA recipe 状态感知 nextActions、scorecard ready 状态感知 trend 修复动作、TUI 运行中产品循环观察命令、TUI running-safe 标记收敛、TUI 运行中 fork 持久化上下文、terminal dry-run 可验收报告、fork dry-run 预览、fork resume 健康检查、resume dry-run 预览、resume 候选去噪、preflight 运行诊断摘要、benchmark status/summary JSON 内嵌 report、resume 低信息澄清会话去噪、resume 当前 workspace 与短任务去噪、privacy 配置化禁用词扫描、fork 默认候选去噪、fork JSON 错误结构化、benchmark 证据 freshness 可见性、SOTA recipe baseline-aware nextActions、scorecard ready baseline-aware nextActions，以及 round ready baseline-aware nextActions，下一轮可继续关注 TUI 可观测性、恢复历史或环境自动化验收的真实交互阻力。

本轮继续补齐 Fork JSON 恢复动作清单，让 fork 成功、预览和错误 JSON 都能提供可直接渲染的恢复/候选动作队列。

## 常用检查命令

提交前建议至少运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
./scripts/deepcli help goal
./scripts/deepcli help plan
./scripts/deepcli help fork
./scripts/deepcli fork --help
./scripts/deepcli help resume
./scripts/deepcli resume --dry-run --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli fork --current --dry-run --json
./scripts/deepcli fork --current --no-open --verify --json
./scripts/deepcli fork --dry-run --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli fork --current --no-open --verify --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4],verification:.verification.status}'
./scripts/deepcli terminal --dry-run --json
DEEPCLI_TERMINAL_APP=iTerm2 ./scripts/deepcli terminal --dry-run --json
TERM_PROGRAM=iTerm.app ./scripts/deepcli terminal --dry-run --json
./scripts/deepcli terminal --app iTerm2 --dry-run --json
./scripts/deepcli --help | rg 'terminal'
./scripts/deepcli --help | rg 'git status'
./scripts/deepcli --help | rg 'git create-branch'
./scripts/deepcli git status --json
./scripts/deepcli git status --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
cargo test git_write
./scripts/deepcli git create-branch feature/preview --dry-run --json
./scripts/deepcli git commit 'preview checkpoint' --dry-run --json
./scripts/deepcli --help | rg 'approval list|approval approve|btw ask|btw answer'
./scripts/deepcli approval list --json
./scripts/deepcli approval list --json | jq '.nextActions'
./scripts/deepcli help approval | rg 'deepcli.approval.action.v1|approval approve'
./scripts/deepcli btw list --json
./scripts/deepcli btw list --json | jq '.nextActions'
./scripts/deepcli help btw | rg 'deepcli.btw.action.v1|btw answer'
./scripts/deepcli sessions -h
./scripts/deepcli preflight --json
./scripts/deepcli review
```

产品循环检查：

```bash
./scripts/deepcli scorecard --json
./scripts/deepcli scorecard --json | jq '{checklist,nextActions}'
./scripts/deepcli scorecard --json | jq '.categories[0] | {title,checklist,nextActions}'
./scripts/deepcli quickstart --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli selftest --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli version --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli doctor --quick --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli doctor shell --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli status --json | jq '{checklist:.checklist[0:4],sessionChecklist:.session.checklist[0:4],nextActions:.session.nextActions[0:4]}'
./scripts/deepcli usage --json | jq '{checklist:.checklist[0:4],sessionChecklist:.session.checklist[0:4],nextActions:.session.nextActions[0:4]}'
./scripts/deepcli config validate --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli config sources --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli credentials status --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli permissions show --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli timeout --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli model show --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli model list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli completion status zsh --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli completion install zsh --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli env check docker --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli env plan docker --smoke --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli test discover --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli test run --json -- 'printf ok' | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli prompt list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli prompt render code-review --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli skill list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli agent list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli approval list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli btw list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli session list --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli session history --json | jq '{kind,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli session search compiler --json | jq '{hitCount,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli session search __definitely_no_deepcli_match__ --json | jq '{hitCount,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli next --json | jq '{checklist:.checklist[0:4],quickLinkChecklist:.quickLinkChecklist[0:4],nextActions:.nextActions[0:4],quickLinks:.quickLinks[0:4]}'
./scripts/deepcli session diagnose --json | jq '{checklist:.checklist[0:4],quickLinkChecklist:.quickLinkChecklist[0:4],recommendedNextActions:.recommendedNextActions[0:4],quickLinks:.quickLinks[0:4]}'
./scripts/deepcli logs --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli diagnose --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli support --json | jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
jq '{checklist:.checklist[0:4],nextActions:.nextActions[0:4]}' .deepcli/support/latest/manifest.json
./scripts/deepcli recipes sota --json
./scripts/deepcli recipes sota --json | jq '{title,summary,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli recipes sota --json | jq '{matches:([.checklist[].command] == .nextActions),checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli recipes sota --json | jq '.checklist[] | select(.command | contains("baseline"))'
./scripts/deepcli round --json
./scripts/deepcli round --json | jq '{checklist,nextActions}'
./scripts/deepcli round --json | jq '.gates[] | {id,status,checklist,nextAction}'
./scripts/deepcli round --json | jq '.scorecard.categories[] | select(.id=="benchmark_evidence") | {checklist,nextActions}'
./scripts/deepcli benchmark status --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli benchmark summary --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli benchmark trends --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli preflight --dry-run --json | jq '{checklist:.checklist[0:4],checks:[.checks[0:4][] | {name,status,command}]}'
./scripts/deepcli gate --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli handoff --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli round --json --run-benchmark --fail-on-command
./scripts/deepcli benchmark baseline-template --output .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json
./scripts/deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
./scripts/deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json
./scripts/deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json | jq '{status,checklist:.checklist[0:4],nextActions:.nextActions[0:4]}'
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

## 本轮新增上下文

121. SOTA recipe 当前动作清单对齐
   - 产品缺口：`deepcli recipes sota --json` 的顶层 `nextActions` 已经是状态感知动作队列，但 `checklist[]` 仍从静态 recipe 命令链生成，外部 UI 只渲染 checklist 时会先展示已读的 recipe/scorecard/round 导航，而不是当前最该执行的动作。
   - 结果：`recipes sota` 顶层 `checklist[]` 改为从状态感知 `nextActions` 派生并逐项对齐；静态完整工作流继续保留在 `recipes[].commands`。
   - 目的：TUI、外部产品循环页和脚本验收可以把 `checklist[]` 当作当前动作按钮队列，不需要自行判断 SOTA recipe 的静态命令和动态动作哪个更该展示。

122. Shell Completion JSON 动作清单
   - 产品缺口：`deepcli completion status/install --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，shell 安装面板、TUI 或外部 onboarding UI 还需要自行给安装、复查和 shell 体检动作命名。
   - 结果：`completion status --json` 与 `completion install --json` 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；缺失、过期、dry-run install 和 up-to-date 场景都保留原 `nextActions` 兼容脚本。
   - 目的：shell completion 的安装、刷新和复查链路可以和 status、usage、doctor、logs、model、fork 等 JSON 面板一致，外部 UI 可直接渲染可点击动作。

123. Test JSON 动作清单
   - 产品缺口：`deepcli test discover/run --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，TUI Tests 面板、外部测试面板或 CI artifact UI 仍要自行给“运行测试、重新发现、验收、gate”动作命名。
   - 结果：`test discover --json` 与 `test run --json` 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；具体测试命令会显示为 `Run test command`，测试帮助显示为 `Open test help`，失败的 run 场景会把重新发现测试标为 `Discover test commands`。
   - 目的：编程后的测试发现、测试执行、验收和交付 gate 可以直接作为按钮队列呈现，推进 deepcli 自发测试能力和外部 UI 可用性。

124. Git Inspect JSON 动作清单
   - 产品缺口：`deepcli git status|diff|branch|message --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，Git 面板和外部编码工作流 UI 仍要自行给 diff、commit message、review、gate 和帮助动作命名。
   - 结果：Git 只读 inspect JSON 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；Git status 场景会直接展示 `Inspect git diff`、`Prepare commit message` 和 `Review current diff`。
   - 目的：编码后的 Git 状态、diff、提交信息建议和 review/gate 链路可以直接作为按钮队列呈现，减少用户在修改后进入验收和提交阶段的手动判断。

125. Prompt Inspect JSON 动作清单
   - 产品缺口：`deepcli prompt list|get|render --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，Prompt 面板、外部 prompt 管理页或脚本化验收仍要自行给打开、渲染和帮助动作命名。
   - 结果：Prompt inspect JSON 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；具体 prompt 场景会展示 `Open prompt`、`Render prompt` 和 `Open prompt help`。
   - 目的：用户在编程前复用、查看和渲染 prompt 时，可以直接从结构化动作队列进入下一步，Prompt 面板不必解析命令字符串。

126. Skill 与 Agent Inspect JSON 动作清单
   - 产品缺口：`deepcli skill list|run --json` 与 `deepcli agent list|show --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，Library 面板、外部插件页或子任务编排页仍要自行给运行 skill、查看子 Agent、列表刷新和帮助动作命名。
   - 结果：Skill/Agent inspect JSON 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；Skill 场景会展示 `Run skill` 和 `List skills`，Agent 场景会展示 `Inspect sub-agent` 和 `List sub-agents`。
   - 目的：Prompt、Skill、Agent 三类能力库 JSON 现在使用一致的结构化动作队列，Library 面板可以直接渲染下一步按钮，不必解析命令字符串。

127. Approval 与 BTW 协作队列 JSON 动作清单
   - 产品缺口：`deepcli approval list|approve|deny|clear --json` 与 `deepcli btw list|answer|clear --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，协作队列面板仍要自行给批准、拒绝、复查、查看全部和帮助动作命名。
   - 结果：Approval/BTW list 和 action JSON 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；Approval pending 场景会展示 `Approve request`、`Deny request`、`Review approvals` 和 `Open approval help`，BTW 场景会展示 `Review by-the-way questions` 和 `Open by-the-way help`。
   - 目的：运行中 TUI 与外部协作 UI 可以直接渲染审批和旁路问题处理闭环，不必解析命令字符串或猜测按钮文案。

128. Session List/Inspect JSON 动作清单
   - 产品缺口：`deepcli session list --json` 与 `deepcli session show|history|summary|tools|tests|diffs|backups --json` 已经是恢复历史页和 TUI 面板的数据源，但缺少顶层 `nextActions` 与 `checklist[]`，外部 UI 只能展示会话数据，不能直接给恢复预览、历史、next、diagnose、列表和帮助动作命名。
   - 结果：Session list JSON 围绕首个展示会话输出 resume preview、history、next、diagnose、list-all、prune-empty dry-run 和 help 动作，并从这些动作派生 checklist；Session inspect JSON 输出同一会话的 resume preview、next、diagnose、session list 和 help 动作，并派生 checklist。
   - 目的：恢复历史页、session 检查页和 TUI 多个观察面板可以直接渲染下一步按钮，不必从 report 文本或 session id 自行拼命令。

129. Environment Inspect JSON 动作清单
   - 产品缺口：`deepcli env check|plan|setup|test ... --json` 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，Environment 面板、安装向导和验收脚本仍要自行给检查、计划、安装、环境测试、accept 和 gate 动作命名。
   - 结果：Environment inspect JSON 从顶层 `nextActions` 派生 `checklist[]`，每项包含 `step`、`label` 和 `command`；未 ready 环境展示 `Set up local environment` 与 `Inspect environment plan`，ready/setup/test 场景展示 `Run environment test`、`Discover test commands`、`Run acceptance checks` 和 `Run delivery gate`。
   - 目的：Docker/编译器环境从预检、安装计划、setup、smoke test 到验收 gate 的链路可以直接作为结构化按钮队列呈现，推进 deepcli 自发环境配置和测试体验。

130. Config/Credentials/Permissions JSON 动作清单
   - 产品缺口：`deepcli config show|sources|validate|get --json`、`credentials status --json`、`permissions show --json` 和 `timeout --json` 位于启动配置、模型切换和安全设置的关键路径，但 JSON 里缺少统一 `checklist[]`，且 credentials/permissions 的 next actions 仍混有 slash-command prose 或空动作。
   - 结果：这些本地 inspect JSON 现在输出可执行 `deepcli ...` 顶层动作，并从动作派生 `checklist[]`；configured credentials 也会给出模型查看、模型列表、配置验证和 doctor，missing/parse-error credentials 会给出具体 provider 的 set/import-env/template 修复动作；permissions 会给出回到 sandbox、配置验证、doctor 和帮助动作；timeout 会给出 usage、trace、inspect、reset 和 help 动作。
   - 目的：设置面板、凭据向导、权限安全页和慢响应排障页可以直接渲染配置/凭据/权限/超时动作按钮，不必解析自然语言或自行拼命令。

131. Benchmark Baseline JSON 动作清单与本地忽略
   - 产品缺口：`scorecard`、`round` 和 `recipes sota` 已经把 `benchmark baseline-template --from-current ... --output .deepcli/baselines/...` 作为 ready 状态下的下一步，但 baseline-template JSON 自身缺少 `checklist[]`；同时 `.deepcli/baselines/` 未被默认忽略，用户执行推荐动作后工作区会出现未跟踪 baseline artifact。
   - 结果：`deepcli benchmark baseline-template --json` 现在从可执行 `nextActions` 派生 `checklist[]`，人工编辑提示仍只保留在 `nextActions`/`report`；`.deepcli/baselines/` 加入 `.gitignore` 和 `.deepignore`。
   - 目的：SOTA baseline 捕获、手工竞品模板和 compare 路径可在外部 UI 中直接渲染动作按钮，同时推荐命令不会污染 Git 工作区或 deepcli 上下文。

132. Benchmark JSON 动作清单全覆盖
   - 产品缺口：`benchmark presets/list/show/clean` 以及 run/run-suite/record artifact 已经输出可执行 `nextActions`，但缺少顶层 `checklist[]`，外部 benchmark 面板仍需要自行给运行、查看、清理和回到 scorecard 的动作命名。
   - 结果：所有带可执行 `nextActions` 的 benchmark JSON 都会派生 `checklist[]`，覆盖 `presets`、`run-suite`、`run`、`record`、`status`、`summary`、`trends`、`baseline-template`、`compare`、`list`、`show` 和 `clean`；`show latest --json` 会对旧 artifact 动态补齐 checklist。
   - 目的：benchmark 证据链的发现、执行、记录、查看、汇总、趋势、baseline、compare 和清理面板可以统一渲染动作按钮，不必解析 report 或硬编码子命令标签。

133. Benchmark Baseline Inventory
   - 产品缺口：ready 状态会推荐生成或维护 `.deepcli/baselines/*.json`，但用户缺少一个只读入口查看当前有哪些 baseline、哪个是默认 competitor、哪些可直接 compare、哪些还需要补 status/durationMs。
   - 结果：新增 `deepcli benchmark baselines --json`，输出稳定 `deepcli.benchmark.baselines.v1`，包含 baselineCount、ready/needs_values/invalid 计数、defaultBaseline、每个 baseline 的 readiness、case 数、缺值数、compare/template nextActions 和 checklist；坏 JSON 会作为 invalid 条目展示，不会让整个列表失败。
   - 目的：SOTA 横向 benchmark 对比工作流从“生成模板后记路径”变成可发现、可检查、可渲染的 baseline inventory 面板，外部 UI 或 TUI 可以直接引导 compare 或补齐模板。

134. Resume Candidate Diagnostics
   - 产品缺口：`deepcli resume --dry-run --json` 在当前 workspace 没有可恢复候选时只能告诉用户失败并跳到 session list；但 session list 可能仍显示大量空会话、tool-only/diagnostic 会话或旧 workspace 会话，用户会误以为历史没有记录或 resume 过滤器出错。
   - 结果：新增 `deepcli resume candidates --json`，输出稳定 `deepcli.resume.candidates.v1`，包含 total/shown/eligible/hidden 计数、默认可恢复候选、每个候选的 activity、eligible、hiddenReason、resumePreviewCommand、nextActions、checklist 和 report；hiddenReason 区分 empty、tool_only_or_diagnostic、low_information_clarification、thin_completed_chat、other_workspace 和其它 non_resumable。`resume --dry-run --json` 无候选错误的首个 nextAction 改为 `deepcli resume candidates --json`。
   - 目的：恢复历史页、fork 源选择和用户手动排障可以直接解释“没有默认可恢复历史”的原因，不必在 `resume --dry-run`、`session list` 和人工猜测之间来回跳。

135. Fork No-Source Candidate Diagnostics
   - 产品缺口：`deepcli fork --dry-run --json` 已复用 resume 的可恢复候选过滤，但无源错误的 `nextActions` 仍优先给出 `deepcli resume --dry-run --json`，在当前 workspace 已知没有默认可恢复候选时会把外部 UI 和用户带回同一个失败入口。
   - 结果：fork 源选择失败时，`deepcli.session.fork.v1` error JSON 和非 JSON report 的一般 no-source 动作优先给出 `deepcli resume candidates --json`，再给出结构化 session list 和文本 session list；shell 中误用 `--current` 的首个动作仍保留 `deepcli fork --dry-run --json`，后续候选诊断也改为 resume candidates。
   - 目的：fork 源选择、恢复历史页和外部 UI 可以直接解释“为什么没有可 fork 的默认上下文”，而不是先执行一个预期会失败的 resume preview。

136. Benchmark Trends Baseline Navigation
   - 产品缺口：`deepcli benchmark trends --json` 在 trends 状态已 ok 或 regression 时固定推荐默认 competitor baseline compare；当 `.deepcli/baselines/competitor.json` 缺失时，这会让用户从趋势页跳到一个不可完成的 compare 路径，而 scorecard、round 和 recipes sota 已经有状态感知 baseline 导航。
   - 结果：`benchmark trends` 的 JSON 和文本 next actions 改为复用 `sota_baseline_next_actions(workspace)`；empty 状态仍先提示采集证据，insufficient_history 状态先提示 `deepcli round --json --run-benchmark --fail-on-command` 补样本，再追加 baseline 导航；其它状态根据默认 baseline 是否 ready 选择 `baseline-template --from-current`、手工 template、baseline inventory 或 compare。
   - 目的：benchmark 趋势页、SOTA 循环页和 baseline inventory 的后续动作保持一致，外部 UI 不会在默认 baseline 缺失时提前展示必然失败的 compare 按钮。

137. Benchmark Exploration Baseline Navigation
   - 产品缺口：`deepcli benchmark presets/list/summary` 仍固定推荐默认 competitor baseline compare；当 `.deepcli/baselines/competitor.json` 缺失时，用户会从 benchmark 探索页进入预期失败的 compare，而不是先捕获 current baseline 或生成手工 competitor template。
   - 结果：`benchmark presets`、`benchmark list` 和 `benchmark summary` 的 JSON 与文本 next actions 改为复用 `sota_baseline_next_actions(workspace)`，并继续从这些动作派生 checklist；默认 baseline 缺失且当前 artifact 可完整捕获时先推荐 `baseline-template --from-current` 和手工 competitor template，文件存在后才推荐 compare。
   - 目的：benchmark 发现、历史列表、汇总、趋势和 SOTA 循环页使用一致的 baseline 状态导航，外部 UI 不会在 baseline 尚未准备好时展示不可执行的 compare 按钮。

138. Scorecard Normalized Score Clarity
   - 产品缺口：`deepcli scorecard --json` 在无 gaps、`percent=100` 时仍输出 `score=80`、`maxScore=80`；用户或外部 UI 如果只读取 `score`，会误以为产品还有 20 分缺口。
   - 结果：`deepcli.scorecard.v1` 和内嵌的 `deepcli.scorecard.summary.v1` 增加 `normalizedScore` 与 `scoreScale`，明确 `score` 是 raw points、`normalizedScore`/`percent` 是 0-100 展示分，并把推荐展示字段标为 `scoreScale.display=normalizedScore`；scorecard 与 round 文本报告也拆清 raw score 和 normalized score。
   - 目的：scorecard、round 和外部产品循环 UI 不再把 raw points 误读成百分制分数，ready 状态下能清楚展示 100/100，而不破坏既有 `score`/`maxScore` 脚本兼容性。

139. Ready Round Product Opportunities
   - 产品缺口：当 `scorecard` 和 `round` 都 ready 且没有 gaps 时，JSON 只剩 preflight、gate 和 baseline 维护动作；外部 UI 难以区分“必须修复的缺口”和“下一轮可继续推进的产品机会”，持续产品循环容易停在 100 分报告上。
   - 结果：`deepcli.scorecard.v1`、`deepcli.scorecard.summary.v1` 和 `deepcli.round.v1` 增加非阻塞 `opportunities[]`，每项包含 id、title、summary、impact、priority、effort、status、nextActions 和 checklist；ready round 文本也展示 opportunities。首批机会聚焦 competitor baseline 准备/对比和 SOTA product loop 体验复查。
   - 目的：ready 状态继续给产品设计师和外部 UI 提供下一轮可选方向，同时不把机会误标成 gaps，不改变 gates、status 或现有 nextActions。

140. SOTA Recipe Product Opportunities
   - 产品缺口：`scorecard` 和 `round` 已经能输出非阻塞 `opportunities[]`，但用户从 `deepcli recipes sota --json` 这个产品循环入口进入时仍只能看到 checklist/nextActions，缺少机会的 summary、impact、effort 和 status。
   - 结果：`deepcli.recipes.v1` 在 `sota` topic 下复用当前 `round` 的 `opportunities[]`，普通 topic 返回空数组；文本模式在有机会时也展示 opportunities。
   - 目的：外部产品循环页、TUI recipe 面板和脚本入口可以在同一份 SOTA recipe 报告里同时渲染动作按钮和机会说明，不必额外调用 round 或自行解释动作原因。

141. Product Opportunities First-Class Entry
   - 产品缺口：`scorecard`、`round` 和 `recipes sota` 已经能输出机会对象，但用户或外部 UI 若只想查看“当前可继续推进的产品机会”，仍必须打开完整 round/recipe 报告并从中抽取 `opportunities[]`。
   - 结果：新增 `deepcli opportunities --json` 与 `/opportunities`/`/opportunity`，输出稳定 `deepcli.opportunities.v1`，复用当前 `round` 的机会对象，顶层提供 `nextActions` 和 `checklist[]`；TUI 运行中也可作为本地只读旁路命令执行。
   - 目的：产品循环页、TUI 面板和脚本可以直接渲染机会卡片和动作按钮，不需要读取完整 round 报告或复制机会判断逻辑。

142. Baseline Template Stdout-Only Next Actions
   - 产品缺口：`deepcli benchmark baseline-template --from-current --json` 在未传 `--output` 时只把 ready baseline 打到 stdout，却会推荐 `compare --baseline .deepcli/baselines/current-main.json`，让用户跳到一个尚未写入的文件。
   - 结果：stdout-only baseline-template 报告现在先推荐带 `--output <path>` 的持久化命令；只有本次实际写入 workspace baseline 文件后，ready baseline 才推荐 compare，needs_values baseline 才推荐编辑目标文件和 compare。
   - 目的：baseline capture、current-main 本地基线和 competitor compare 链路保持可执行，不会从预览 JSON 直接跳到不存在的文件。

143. Current Baseline Ready Navigation
   - 产品缺口：生成 ready 的 `.deepcli/baselines/current-main.json` 后，`scorecard`、`round`、`recipes sota`、`opportunities` 和 benchmark 探索入口仍会重复推荐 current capture，而不是推进到尚缺的 competitor baseline。
   - 结果：共享的 baseline 导航现在会先检查 current-main baseline 文件是否覆盖 required cases 且都有 `status`/`durationMs`；若默认 competitor baseline 缺失但 current-main 已 ready，只推荐生成 `.deepcli/baselines/competitor.json` 的手工模板。
   - 目的：SOTA baseline 工作流从“捕获 current -> 准备 competitor -> compare”向前推进，不会在 current baseline 已 ready 后继续重复同一步。

144. Benchmark Baselines Needs Default State
   - 产品缺口：当 `.deepcli/baselines/current-main.json` 已 ready、但默认 `.deepcli/baselines/competitor.json` 缺失时，`deepcli benchmark baselines --json` 会把 inventory 判为 ready，并先推荐 compare 非默认 current-main baseline，容易让用户误以为 SOTA 对照基线已准备好。
   - 结果：`benchmark baselines` 现在返回 `status=needs_default`，`defaultBaseline.present=false`，首个 `nextActions` 和 `checklist[0]` 都指向生成 competitor baseline template；current-main compare 仅保留为后续辅助动作，且不会重复推荐 current capture。
   - 目的：baseline inventory 与 scorecard、round、recipes 和 opportunities 的状态推进一致，先补默认 competitor 对照，再进入 compare。

145. Opportunity Baseline Inventory First
   - 产品缺口：`opportunities --json` 的 baseline 机会会直接把写入 `.deepcli/baselines/*.json` 的命令放在首位，用户或外部 UI 在点击前看不到当前 baseline inventory 是 empty、needs_default 还是 ready。
   - 结果：baseline 机会现在先给 `deepcli benchmark baselines --json`，再给 current capture、competitor template 或 competitor compare；顶层 opportunities `nextActions` 和 `checklist[]` 也从该只读 inventory 动作开始。
   - 目的：机会页从“直接写本地 evidence artifact”变成“先检查状态，再执行下一步”，降低误操作并让 UI 能先渲染 baseline inventory。

146. Ready Reports Link Opportunities
   - 产品缺口：`scorecard`、`round` 和 `recipes sota` 已经输出 `opportunities[]`，但顶层 `nextActions` 没有 `deepcli opportunities --json`，用户或外部 UI 想单独打开机会页仍要解析完整报告或记住命令。
   - 结果：ready scorecard、ready round 和 SOTA recipe 的顶层动作现在包含 `deepcli opportunities --json`；round 仍保留 preflight/gate 在前，scorecard 的 benchmark 修复动作也保持优先。
   - 目的：机会页成为可发现的一等产品循环入口，而不是只作为嵌套字段存在。

147. Benchmark Status Baseline Inventory Link
   - 产品缺口：`deepcli benchmark status --json` 在 benchmark evidence ready 时只给 SOTA recipe、presets、run、gate、summary 和 trends，用户已证明证据 ready 后还要从其它入口绕到 baseline inventory。
   - 结果：ready benchmark status 的顶层 `nextActions` 现在会在 `deepcli recipes sota --json` 后直接给出 `deepcli benchmark baselines --json`，并从同一动作派生 `List benchmark baselines` checklist；missing、weak、incomplete、failing 和 stale 状态仍优先展示证据修复路径。
   - 目的：benchmark status 页从“证据体检”自然推进到“查看对照基线”，让 TUI、外部 benchmark 面板和脚本用户先看只读 inventory，再决定生成 baseline、compare 或重新跑 preset。

148. SOTA Baseline Ready-Gated Compare
   - 产品缺口：用户按推荐生成 `.deepcli/baselines/competitor.json` 手工模板后，该文件已经存在但仍是 `needs_values`；共享 SOTA baseline 导航会把“文件存在”误判成“可 compare”，让 scorecard、round、recipes、opportunities 和 benchmark 探索入口提前显示 compare。
   - 结果：`sota_baseline_next_actions` 现在只有在默认 competitor baseline 覆盖 required presets 且每个 case 都有 `status`/`durationMs` 时才返回 compare；默认 baseline 存在但缺值或无效时返回只读 `deepcli benchmark baselines --json`，让 inventory 展示 `needs_values`、人工编辑提示和后续 compare。
   - 目的：SOTA 横向对比从“模板文件已写入”推进到“baseline 已可比”之前，不再误导用户直接 compare，外部 UI 可以先渲染 inventory 状态和缺值说明。

149. Product Opportunity Effort Signals
   - 产品缺口：ready 后的 `opportunities[]` 已经包含 summary、impact 和动作清单，但 `effort` 为空，用户和外部 UI 无法同时判断收益与执行成本，也无法把机会卡片按轻重缓急排序。
   - 结果：`ScorecardOpportunity` 增加稳定 `effort` 字段，并在 scorecard、round、recipes sota 和 opportunities JSON/text 中复用；准备 competitor baseline 标记为 `medium`，已有 ready baseline 的 compare 和产品循环体验复查标记为 `low`。
   - 目的：产品循环入口从“下一步按钮列表”进一步变成可决策的机会队列，帮助用户在 gates 全绿后继续选择最高价值的迭代方向。

150. Benchmark Freshness Opportunity
   - 产品缺口：benchmark evidence 已 ready 但 freshness 为 aging 时，`round` 和 `scorecard` 会把刷新命令放进顶层 nextActions，但 `opportunities[]` 没有解释这个非阻塞刷新机会，用户或外部 UI 只能看到命令而看不到收益、成本和原因。
   - 结果：`scorecard_product_opportunities` 现在在 freshness refreshRecommended 时优先输出 `benchmark_freshness` 机会，包含 summary、impact、`effort=low`、刷新命令和 benchmark status 复查动作；scorecard、round、recipes sota 和 opportunities 入口复用同一机会对象。
   - 目的：ready 状态的 aging benchmark evidence 不再只是隐藏在 nextActions 中的刷新命令，而是可展示、可排序、可解释的产品机会，同时保持 ready/gate 语义不变。

151. Product Opportunity Priority Signals
   - 产品缺口：`opportunities[]` 已经包含 impact 和 effort，但外部 UI 若要判断“先做哪个机会”仍只能依赖数组顺序，无法在卡片、筛选或自动化脚本中展示稳定优先级。
   - 结果：`ScorecardOpportunity` 增加稳定 `priority` 字段，并在 scorecard、round、recipes sota 和 opportunities JSON/text 中复用；benchmark freshness 与 competitor baseline 机会标记为 `high`，产品循环体验复查标记为 `medium`。
   - 目的：机会页从“有序按钮列表”进一步变成可解释的产品决策队列，让 UI 能同时展示收益、优先级和成本，而不需要反推数组排序。

152. Product Opportunity Summary Fields
   - 产品缺口：机会对象已经有 `priority`，但外部 UI 仍要扫描 `opportunities[]` 才能选出主推荐机会和统计优先级分布，产品循环页头和主 CTA 还缺稳定字段。
   - 结果：scorecard、round、recipes sota 和 opportunities JSON 增加共享的 `recommendedOpportunity` 和 `opportunityPriorityCounts`；推荐机会复用当前有序机会列表第一项，计数固定输出 high、medium、low 和 other。
   - 目的：外部 UI、TUI 面板和脚本可以直接渲染主推荐、优先级摘要和机会列表，不需要复制排序或计数逻辑。

153. Product Opportunity Text Summaries
   - 产品缺口：上一轮补齐了 JSON 的 `recommendedOpportunity` 和 `opportunityPriorityCounts`，但文本模式仍只展示完整机会列表，终端用户需要自己从多条机会里判断主推荐和优先级分布。
   - 结果：scorecard、round、recipes sota 和 opportunities 文本输出在机会列表前复用同一摘要，展示 `recommended opportunity: <id> (<priority>, <effort>)` 和 `priority counts: high=... medium=... low=... other=...`。
   - 目的：不使用 JSON 的终端用户也能直接看到本轮主推荐和机会分布，产品循环入口的人工可读体验与结构化 UI 能力保持一致。

154. Product Opportunity Priority Filter
   - 产品缺口：机会页已经有 priority、推荐机会和计数，但 `/opportunities` 仍只能返回全量机会；终端用户和外部 UI 若只想处理 high 或 medium 机会，必须自行扫描数组并重新生成动作队列。
   - 结果：`/opportunities` 和 `deepcli opportunities` 增加 `--priority high|medium|low|other`；过滤会同步影响 `opportunities[]`、`recommendedOpportunity`、当前 `opportunityPriorityCounts`、`nextActions` 和 `checklist[]`，同时 JSON 保留 `totalOpportunityCount`、`filteredOutOpportunityCount` 和全量 `availablePriorityCounts`。
   - 目的：机会页可以直接聚焦某个优先级的可执行动作，TUI/外部 UI 能用同一入口实现优先级 tab 或筛选按钮，而不需要复制 deepcli 的机会排序与动作展开逻辑。

155. Resume Candidate Recovery Actions
   - 产品缺口：`resume candidates --json` 已经能解释 eligible/hidden 和隐藏原因，但当当前 workspace 只有空会话或工具/诊断型会话时，顶层动作仍先打开完整列表，用户需要自己推断应该先清理空会话或查看诊断。
   - 结果：没有 eligible 候选时，若存在 empty 候选，`nextActions[0]` 现在是 `deepcli session prune-empty --dry-run --json`；若存在 tool-only 或 non-resumable 候选，会追加 `deepcli session diagnose --limit 5 --json`；后续仍保留结构化 session list、history 和 resume help，`checklist[]` 与动作同步。
   - 目的：恢复页和 fork 失败页能直接把“没有可恢复历史”转化为安全清理和诊断按钮，减少用户误以为历史丢失的阻力。

156. Fork No-Source Recovery Actions
   - 产品缺口：上一轮让 `resume candidates --json` 能直接给出空会话清理和诊断动作，但 `fork --dry-run --json` 在没有可分支源会话时仍只推荐候选页和 session list，用户从“无法打开同样上下文的新终端”到“清理/诊断原因”还要多跳一次。
   - 结果：`no_resumable_context` 的 fork 错误路径现在读取同一套 resume candidates 隐藏原因，先输出 `deepcli session prune-empty --dry-run --json` 和/或 `deepcli session diagnose --limit 5 --json`，再保留 `deepcli resume candidates --json`、结构化 session list 和 `deepcli sessions --all --limit 20`；`--current` 无 active session 的误用路径仍保持 `deepcli fork --dry-run --json` 为首项。
   - 目的：fork 失败页可以直接解释和修复“没有可分支上下文”的本地原因，外部 UI 不必先跳到候选页再生成同样的清理/诊断按钮。

157. Session Prune-Empty JSON Checklist
   - 产品缺口：fork/resume 无可恢复上下文时会推荐 `deepcli session prune-empty --dry-run --json`，但该 JSON 没有 `checklist[]`，且确认动作回到非 JSON 的 `deepcli cleanup sessions --force`，外部 UI 还要自行命名按钮并切换输出形态。
   - 结果：`deepcli.session.prune_empty.v1` 现在输出从 `nextActions` 派生的 `checklist[]`；dry-run 有候选时首项为 `deepcli session prune-empty --force --json`，列表动作改为 `deepcli session list --all --json`；force 标签显示为 `Delete empty sessions`。
   - 目的：恢复页、fork 失败页和历史清理页可以在同一 JSON 工作流内完成预览、确认删除和复查，不需要手工解析 report 或自行命名动作按钮。

158. Round Baseline Inventory First
   - 产品缺口：`round --json` ready 后的顶层 baseline 动作会直接推荐 `baseline-template --from-current` 或手工 competitor template，用户或外部 UI 还没看 baseline inventory 就可能写 `.deepcli/baselines/*` 本地 artifact；而 opportunities 和 benchmark status 已经先给只读 `deepcli benchmark baselines --json`。
   - 结果：ready round 顶层 baseline 队列现在复用 `opportunity_baseline_next_actions`，在 `deepcli opportunities --json` 后先输出 `deepcli benchmark baselines --json`，再输出 current capture、competitor template 或 compare；`round.checklist[]` 与同一队列对齐。
   - 目的：产品循环页从 ready round 进入 SOTA baseline 工作流时先展示 baseline inventory 状态，减少误写本地 evidence artifact，并和 opportunities/benchmark status 的导航一致。

159. Scorecard Baseline Inventory First
   - 产品缺口：`scorecard --json` ok 后的顶层 baseline 动作仍会直接推荐 `baseline-template --from-current`、手工 competitor template 或 compare；用户或外部 UI 从评分页进入 SOTA baseline 工作流时，还没看到 baseline inventory 就可能写本地 artifact，和 round/opportunities/benchmark status 的导航不一致。
   - 结果：ok scorecard 顶层 baseline 队列也复用 `opportunity_baseline_next_actions`，在 benchmark status 后先输出 `deepcli benchmark baselines --json`，再输出 current capture、competitor template 或 compare；`scorecard.checklist[]` 与同一队列对齐。
   - 目的：评分页、产品循环页和机会页都先展示 baseline inventory 状态，再进入写入或对比动作，减少入口差异和误操作。

160. Opportunity Effort Filter
   - 产品缺口：机会对象已经有 `effort` 字段，但 `/opportunities` 只能按 priority 过滤；用户或外部 UI 想先处理低成本机会时仍要自己扫描数组、重建推荐机会和动作队列。
   - 结果：`/opportunities` 和 `deepcli opportunities` 增加 `--effort high|medium|low|other`，可与 `--priority` 组合取交集；JSON 增加 `opportunityEffortCounts` 和 `availableEffortCounts`，文本摘要增加 `effort counts`，过滤会同步影响 `opportunities[]`、`recommendedOpportunity`、当前计数、`nextActions` 和 `checklist[]`。
   - 目的：机会页可以直接聚焦低成本或指定成本的动作队列，TUI/外部 UI 能用同一入口实现成本 tab 或快捷筛选，不需要复制 deepcli 的机会过滤逻辑。

161. SOTA Recipe Effort Counts
   - 产品缺口：`opportunities --json` 已经能输出 `opportunityEffortCounts`，但更常用的 `recipes sota --json` 入口只给 `opportunityPriorityCounts`，外部产品循环页若想展示成本分布仍要扫描 `opportunities[]` 或再次调用机会页。
   - 结果：`deepcli.recipes.v1` 在 SOTA topic 下增加顶层 `opportunityEffortCounts`，复用同一套 high、medium、low、other 计数；文本模式继续通过共享机会摘要展示 `effort counts`。
   - 目的：SOTA recipe 成为完整的产品循环入口，UI 和脚本可以直接展示主推荐、优先级分布和成本分布，不必复制机会统计逻辑。

162. Scorecard Opportunity Effort Counts
   - 产品缺口：SOTA recipe 和 opportunities 入口已经能输出 `opportunityEffortCounts`，但 `scorecard --json` 与 `round --json` 内嵌的 `scorecard` 摘要仍只有 `opportunityPriorityCounts`，评分页和 round 页的 UI 还要扫描机会数组才能展示成本分布。
   - 结果：`deepcli.scorecard.v1`、`deepcli.scorecard.summary.v1` 和 `deepcli.round.v1` 顶层都增加 `opportunityEffortCounts`，复用同一套 high、medium、low、other 计数。
   - 目的：scorecard、round、recipes 和 opportunities 四个产品循环入口都能直接展示主推荐、优先级分布和成本分布，减少外部 UI 的重复统计逻辑。

163. Product Opportunities Checklist Label
   - 产品缺口：ready round 和 SOTA recipe 已经会把 `deepcli opportunities --json` 放进顶层动作队列，但 `local_action_checklist` 没有专用标签，外部 UI 和 TUI 渲染按钮时只能显示泛化的 `Run command`。
   - 结果：所有 `deepcli opportunities ...` 动作现在通过共享 checklist 标签显示为 `Open product opportunities`。
   - 目的：产品循环入口的 checklist 按钮更接近用户意图，用户能直接看出这是打开机会页，而不是执行不明泛化命令。

164. Ready Round SOTA Recipe Link
   - 产品缺口：`round --json` 是主产品循环状态页，但 ready 状态的顶层动作只有 preflight、gate、opportunities 和 baseline workflow；用户或外部 UI 想从 ready round 打开完整 SOTA playbook 时，需要记住 `deepcli recipes sota --json` 或先绕到机会页。
   - 结果：ready round 顶层 `nextActions` 现在在 preflight/gate 后加入 `deepcli recipes sota --json`，再进入 `deepcli opportunities --json` 和 baseline inventory/template/compare 队列；`checklist[]` 同步显示 `Open SOTA product loop recipe`。`recipes sota --json` 复用 round 队列时会过滤这个自引用，避免 recipe 自己推荐打开自己。
   - 目的：round 主状态页能直接连接完整产品循环 playbook 和当前机会页，减少 ready 状态下的入口断裂。

165. Benchmark Gate Freshness Action
   - 产品缺口：benchmark evidence 已 ready 但 freshness 为 aging/stale 时，round 顶层动作和机会对象会提示刷新，但 `benchmark_evidence` gate 自己仍给 `deepcli benchmark summary --json`，按 gate 卡片渲染的 UI 看不到刷新按钮。
   - 结果：当 `benchmark_freshness_refresh_action` 存在时，`benchmark_evidence` gate 的 `nextAction` 改为 `deepcli round --json --run-benchmark --fail-on-command`，`checklist[]` 同步显示 `Refresh benchmark evidence`；freshness 不需要刷新时仍保留 summary 动作。
   - 目的：round 全局动作、机会对象和 gate-level 按钮在 aging/stale benchmark evidence 上保持一致，用户从任一视图都能直接刷新证据。

166. Baseline Inventory Summary
   - 产品缺口：`deepcli benchmark baselines --json` 是 SOTA baseline 工作流第一站，但外部 UI 若要展示默认 competitor baseline 是否可 compare、主推荐动作和按钮标签，仍要扫描多个字段或解析 `report`。
   - 结果：`deepcli.benchmark.baselines.v1` 增加顶层 `summary`，包含 status、baseline/ready/needs_values/invalid 计数、默认 baseline path/status、默认 compare readiness、可 compare baseline 数量，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：baseline inventory 可以直接渲染页头、状态徽标和主 CTA，不再复制 deepcli 的 action/checklist 推导逻辑。

167. Benchmark Trends Summary
   - 产品缺口：`deepcli benchmark trends --json` 是 round 趋势 gate 的数据源，但趋势页头若要展示 regression、stable pass、slower/faster 等概览和主推荐按钮，仍需要扫描 `trends[]` 或解析 `report`。
   - 结果：`deepcli.benchmark.trends.v1` 增加顶层 `summary`，包含 status、artifact/case 数、regression/recovered/stable pass 计数、slower/faster/flat/unknown duration 计数，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：趋势页、round gate 详情和外部 UI 可以直接渲染核心趋势结论和主 CTA，不再复制趋势聚合逻辑。

168. Benchmark Status Summary
   - 产品缺口：`deepcli benchmark status --json` 是 benchmark evidence 的主解释页，但页头若要展示 readiness、freshness、required preset 覆盖和刷新按钮，仍需要拼接 `totals`、`meaningful`、`freshness`、`presetCoverage`、`checklist` 或解析 `report`。
   - 结果：`deepcli.benchmark.status.v1` 顶层和 `round --json` 内嵌 `benchmarkStatus` 增加 `summary`，包含 status/ready、artifact/meaningful 计数、freshness 状态与年龄、refresh action、required preset 覆盖计数、gapCount，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：benchmark evidence 页和 round gate 详情可以直接渲染证据页头和主 CTA，不再复制 status 聚合与 checklist 标签逻辑。

169. Benchmark Summary Header
   - 产品缺口：`deepcli benchmark summary --json` 是 benchmark 历史汇总页，但页头若要展示 artifact/case 数、通过率、失败/记录计数和主推荐按钮，仍需要重新汇总 `cases[]` 或解析 `report`。
   - 结果：`deepcli.benchmark.summary.v1` 增加顶层 `summary`，包含 status、artifact/case 数、total/executable/passed/failed/timeout/recorded/other 计数、passRatePercent，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：历史汇总页和外部 UI 可以直接渲染核心历史指标和主 CTA，不再复制 summary 聚合逻辑。

170. Benchmark Presets Summary
   - 产品缺口：`deepcli benchmark presets --json` 是刷新 benchmark 证据前的 preset picker，但页头和外部 UI 若要展示默认 suite、required evidence preset、optional preset 和主推荐按钮，仍需要扫描 `presets[]` 或硬编码 preset 名称。
   - 结果：`deepcli.benchmark.presets.v1` 增加顶层 `summary`，包含 status、presetCount、defaultSuitePresetCount、requiredEvidencePresetCount、optionalPresetCount、defaultSuiteAction、defaultSuitePresets、requiredEvidencePresets，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`；每个 preset 条目也标出 `defaultSuite` 与 `requiredEvidence`。
   - 目的：证据采集页、TUI benchmark 面板和脚本可以直接渲染默认必跑项、可选项和主 CTA，不再复制 preset 分类逻辑。

171. Benchmark Cleanup Summary
   - 产品缺口：`deepcli benchmark clean --dry-run --json` 是删除本地 benchmark evidence 前的确认页，但页头若要展示候选数、保留策略、是否会真实删除以及确认按钮，仍需要拼接多个字段或解析 `report`。
   - 结果：`deepcli.benchmark.cleanup.v1` 增加顶层 `summary`，包含 status、dryRun、force、artifactCount、candidateCount、deletedCount、keep、olderThanDays、all、willDelete，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：清理确认页、TUI benchmark 维护面板和脚本可以直接渲染删除风险、保留策略和主 CTA，不再复制 cleanup 聚合逻辑。

172. Benchmark List Summary
   - 产品缺口：`deepcli benchmark list --json` 是本地 benchmark artifact 列表页，但页头若要展示 artifact 总数、最新证据和主推荐按钮，仍需要扫描 `artifacts[]` 或复制 checklist 推导逻辑。
   - 结果：`deepcli.benchmark.list.v1` 增加顶层 `summary`，包含 status、artifactCount、latestArtifactPath、latestCreatedAt、latestSuite、latestCase、latestPreset、latestStatus，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：artifact 列表页、TUI benchmark 面板和脚本可以直接渲染最新证据、总量和主 CTA，不再复制 list 聚合逻辑。

173. Benchmark Artifact Detail Summary
   - 产品缺口：`deepcli benchmark show latest --json` 是 artifact 详情页入口，但页头若要展示状态、suite/case/preset、执行模式、命令数、耗时和主推荐按钮，仍需要解析 `execution`、`declaredCommands` 或复制 checklist 推导逻辑。
   - 结果：`deepcli.benchmark.record.v1` 增加顶层 `summary`，包含 status、artifactPath、createdAt、suite、case、preset、mode、ranByDeepcli、commandCount、durationMs，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`；record/run 写入新 artifact 时生成 summary，show 读取旧 artifact 时动态补齐 summary 和 checklist。
   - 目的：artifact 详情页、TUI benchmark 面板和脚本可以直接渲染详情页头和主 CTA，不再复制 artifact 执行摘要聚合逻辑。

174. Round Product Loop Summary
   - 产品缺口：`deepcli round --json` 是主产品循环状态页，但页头若要展示 ready/status、scorecard 分数、benchmark freshness、gate/gap/opportunity 计数和主推荐按钮，仍需要解析 `report`、扫描 `gates[]`/`opportunities[]` 或复制 checklist 推导逻辑。
   - 结果：`deepcli.round.v1` 增加顶层 `summary`，包含 status、ready、scoreThreshold、scorecardPercent、benchmarkStatus、benchmarkFreshnessStatus、benchmarkFreshnessAgeSeconds、benchmarkFreshnessAge、benchmarkRefreshRecommended、gateCount、passedGateCount、failedGateCount、gapCount、opportunityCount、recommendedOpportunityId，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：产品循环页、TUI round 面板和脚本可以直接渲染本轮状态页头和主 CTA，不再复制 round 聚合与动作命名逻辑。

175. Opportunities Page Summary
   - 产品缺口：`deepcli opportunities --json` 是产品机会页入口，但页头若要展示筛选条件、当前/总/过滤掉的机会数、推荐机会和主推荐按钮，仍需要扫描 `opportunities[]`、解析 `filter` 或复制 checklist 推导逻辑。
   - 结果：`deepcli.opportunities.v1` 增加顶层 `summary`，包含 status、ready、priorityFilter、effortFilter、opportunityCount、totalOpportunityCount、filteredOutOpportunityCount、recommendedOpportunityId，以及从 checklist 派生的 `recommendedAction` 和 `recommendedActionLabel`。
   - 目的：机会页、TUI product opportunities 面板和脚本可以直接渲染筛选后的机会页头和主 CTA，不再复制机会筛选摘要与动作命名逻辑。

176. Command Registry 显式元数据
   - 产品缺口：harness 重构要求主要命令入口由 registry 驱动，但公开命令的分组、running-safe 标记与 completion-only 顶层别名仍由分散硬编码推导；新增 help topic 或 completion alias 时缺少“必须登记 metadata”的契约，后续容易让 help、completion、UI 和 docs 分类重新漂移。
   - 结果：`src/commands/registry.rs` 增加显式 `CommandMetadata` 表、slash alias metadata 和 completion-only alias metadata，记录每个公开命令/兼容 alias/补全别名的 group、running-safe、canonical 或 summary；`CommandRouter::command_metadata()`、`CommandRouter::command_alias_metadata()` 和 `CommandRouter::completion_alias_metadata()` 暴露这些表，`parser`、`help_summaries`、help 详情和 completion catalog 从 registry 读取 metadata；新增 `command_registry_explicitly_owns_public_command_metadata`、`command_registry_owns_slash_alias_metadata` 与 `command_registry_owns_completion_alias_metadata` 契约测试，确保每个 help topic、parser 兼容 alias 和 completion-only alias 都有显式 metadata。
   - 目的：推进 harness 阶段 2/5 的命令 registry 与 docsync 收敛，为后续继续拆 UI running-safe 投影提供单一元数据来源。

177. TUI Running-Safe 提示消费 Command Registry
   - 产品缺口：TUI 运行中 unsupported/deferred 提示仍维护一份手写可用命令列表，运行中 slash palette priority 也用手写 match，容易和 `CommandHelpSummary.running_safe`、slash palette 和 parser 支持范围漂移；例如 `/git`、legacy `/cleanup` 已在 registry 中标记为 running-safe，但提示列表或优先级表不会自动覆盖它们。
   - 结果：`src/ui.rs` 新增 running-safe command hint projection，从 `CommandRouter::help_summaries()` 过滤 `running_safe=true` 生成运行中可用命令集合，只在 UI 层为 `/preflight`、`/git`、`/session`、`/cleanup`、`/btw` 补充 `--dry-run` 或 read-only 等交互限制标签；unsupported 与 deferred 两处提示共用该 projection；running palette priority 从 match 改为 `RUNNING_SAFE_PALETTE_PRIORITY` 显式 projection，覆盖 `/git` 与 `/cleanup`；新增 `running_tui_unsupported_hint_covers_registry_running_safe_commands` 和 `running_safe_palette_priority_covers_registry_running_safe_commands` 单测，约束提示集合与优先级 projection 覆盖 command registry metadata。
   - 目的：推进 harness 阶段 7 的 UI projection 收束，减少 UI 自维护命令清单导致的漂移；完整 UI projection model、主视图收束和剩余大文件拆分仍未完成。

178. Commands 入口测试模块外置
   - 产品缺口：阶段 6 要求旧大文件迁移后只保留 registry、薄适配层或 re-export，但 `src/commands.rs` 仍内联 1.5 万行以上命令契约测试，导致入口文件难以阅读，也让后续 handler/helper 拆分时上下文噪声过大。
   - 结果：将内联 `#[cfg(test)] mod tests { ... }` 机械迁移到 `src/commands/tests.rs`，并把测试专用 import 下沉到该外置测试模块，`src/commands.rs` 仅保留 `#[cfg(test)] mod tests;`；新增 `commands_entrypoint_uses_external_test_module` 契约测试，防止大型命令测试模块或测试专用 import 回流到入口文件；`docs/HARNESS.md` 与 `docs/MODULES/commands.md` 同步记录新的测试归属。
   - 目的：推进 harness 阶段 6 的模块化和测试分层落地，让 `src/commands.rs` 更接近分发/re-export/shared-helper 入口；共享 helper 仍需后续继续收束。

179. Git Identity Helper 拆分
   - 产品缺口：`src/commands.rs` 在 handler 和测试模块拆出后仍直接拥有 Git identity 报告、summary/JSON 投影和只读 Git stdout helper；这些逻辑被 doctor、selftest、privacy、preflight 和 product loop 复用，不属于命令入口分发职责。
   - 结果：新增 `src/commands/git_identity.rs`，迁移 `GitIdentityReport`、`build_git_identity_report`、`format_git_identity_summary`、`git_identity_json`、`git_stdout` 和 `git_stdout_bytes`；`src/commands.rs` 只通过 `pub(crate) use` 重新导出兼容现有 sibling module 调用；`commands_module_docs_cover_split_source_files` 增加该文件，确保 owner 文档继续覆盖 split source。
   - 目的：继续推进 harness 阶段 6 的旧大文件收束，把可独立复用的 Git identity 领域 helper 从入口文件中移出；剩余共享 helper 仍需后续按真实调用链继续拆分。

180. Delivery Diff Source 可见性 Warning 清理
   - 产品缺口：每次 `cargo test`、`cargo check` 和 `deepcli round` 都输出 `SessionDiffSource` 比 `VerificationDiffSource::Session` 更私有的 `private_interfaces` warning，降低 gate 输出信噪比。
   - 结果：将 `src/commands/delivery.rs` 中的 `SessionDiffSource` 提升为 `pub(crate)`，字段仍保持私有；`cargo check` 不再输出该 warning。
   - 目的：清理阶段 6 重构后的编译噪声，让后续验证输出更适合作为产品 gate 证据。

181. Commands 无状态共享 Helper 拆分
   - 产品缺口：阶段 6 要求旧大文件迁移后只保留 registry、薄适配层或 re-export，但 `src/commands.rs` 仍直接拥有参数读取、正整数解析、nextActions 去重、路径展示、provider env key、配置路径、默认模型展示、JSON/text 截断与测试命令展示等无状态 helper，导致入口文件继续承担共享工具库职责。
   - 结果：新增 `src/commands/shared.rs`，迁移 `active_default_model`、`project_config_path`、`workspace_relative_display`、`dedup_preserve_order`、`provider_env_key`、`required_arg`、`parse_positive_usize` 以及相关展示/截断 helper；`src/commands.rs` 只通过 `pub(crate) use` 重新导出兼容现有 sibling module 调用；新增 `commands_entrypoint_delegates_stateless_shared_helpers` 契约测试，防止这些 helper 回流到入口文件并要求 `docs/MODULES/commands.md` 记录 owner。
   - 目的：继续推进 harness 阶段 6 的旧大文件收束，让 `src/commands.rs` 更接近 router/re-export 入口；剩余会话活动、会话存储、环境 nextActions 等 helper 仍需后续按真实调用链继续拆分。

182. Commands 会话共享 Helper 拆分
   - 产品缺口：无状态 helper 拆出后，`src/commands.rs` 仍直接拥有会话列表展示、会话活动回退选择、会话状态名和会话目录大小计算；这些逻辑被 runtime、session、status、usage、resume 和 fork 复用，不属于命令入口分发职责。
   - 结果：新增 `src/commands/session_helpers.rs`，迁移 `format_session_list`、`session_has_no_recorded_activity`、`latest_session_with_recorded_activity`、`session_state_name` 和 `session_storage_bytes`；`src/commands.rs` 继续 re-export `format_session_list` 兼容 runtime 调用，并通过 `pub(crate) use` 兼容现有 sibling module 调用；新增 `commands_entrypoint_delegates_session_shared_helpers` 契约测试，防止会话 helper 回流到入口文件并要求 `docs/MODULES/commands.md` 记录 owner。
   - 目的：继续推进 harness 阶段 6 的旧大文件收束；`src/commands.rs` 当前只剩环境 nextActions 相关 helper 仍需后续拆分。

183. Commands 环境 Action Helper 拆分
   - 产品缺口：会话 helper 拆出后，`src/commands.rs` 只剩环境报告 nextActions、默认环境修复动作和 slash action 到 shell command 转换这组三个 helper；它们被 doctor/env 链路复用，但不属于 router 分发职责。
   - 结果：新增 `src/commands/environment_actions.rs`，迁移 `environment_next_actions`、`default_environment_next_actions` 和 `shell_command_from_slash_command`；`src/commands.rs` 只 re-export `environment_next_actions` 兼容现有 doctor 调用；新增 `commands_entrypoint_delegates_environment_action_helpers` 契约测试，防止环境 action helper 回流到入口文件并要求 `docs/MODULES/commands.md` 记录 owner。
   - 目的：完成本轮针对 `src/commands.rs` 剩余共享 helper 的收束，使入口文件更接近 router/re-export/test-module 声明；阶段 6 仍需继续复查命令面删除/降级策略和部分大命令模块瘦身。

184. TUI Core Session/Context 主视图
   - 产品缺口：阶段 7 建议的主视图包含 Session 与 Context，但 monitor core tabs 只有 Overview/Changes/Tools/Tests/Approvals；会话状态、计划、队列和上下文 cache 仍主要散落在 Overview、Usage、Environment 等视图里，主标签条缺少直接入口。
   - 结果：`MonitorTab` 增加 core `Session` 与 `Context`，core 顺序变为 Overview/Changes/Tools/Tests/Session/Approvals/Context；Session 视图消费 `SessionMonitor.observation` 与 recent events 展示会话状态、计划进度、审批/旁路队列、工具计数和最近事件；Context 视图消费 `SessionMonitor.usage` 与 recent environment 展示 cache hit/miss、请求大小、token 与最近环境摘要；新增/更新 `monitor_tabs_lead_with_core_views_then_advanced`、`monitor_tab_cycles_without_touching_message_input` 与 `task_monitor_tabs_format_usage_tests_environment_approvals_and_trace` 覆盖 tab projection、键盘循环和渲染内容。
   - 目的：继续推进 harness 阶段 7 的主视图收束，让 UI 直接消费已有 `SessionMonitor` projection 展示核心状态；完整 UI projection model、真实终端观感验证和部分 formatter 拆分仍需后续推进。

185. TUI Monitor Metadata 与静态 Quick Actions Projection
   - 产品缺口：Session/Context 加入 core tabs 后，tab 顺序、label、tier 与静态快捷操作仍分散在 `MonitorTab::all()`、`tier()`、`label()` 和 `monitor_quick_actions_for_tab()` 的大 `match` 中；后续新增或降级 monitor tab 时，排序、折叠、渲染名称和快捷操作容易再次漂移。
   - 结果：新增 `MonitorTabMetadata` / `MONITOR_TAB_METADATA`，让 `MonitorTab::all()`、`tier()` 和 `label()` 从同一 projection 派生；新增 `MonitorQuickActionTemplate`、`MonitorTabQuickActions` 与 `MONITOR_STATIC_QUICK_ACTIONS`，让 Overview/Result/Changes/Usage/Tests/Session/Approvals/Context/Trace 的静态快捷操作从 projection 派生，Tools/Health/Library/Deliver/Environment 继续保留动态函数；新增 `monitor_tab_metadata_is_projection_source` 和 `monitor_static_quick_actions_are_projection_source` 单测约束投影来源。
   - 目的：继续推进 harness 阶段 7 的 UI projection model，把 monitor catalog 与静态操作从分散 match 收束为稳定投影；动态 quick actions、formatter 拆分和真实终端观感仍需后续推进。

186. UI Monitor Projection Owner 拆分
   - 产品缺口：`src/ui.rs` 仍直接拥有 monitor tab catalog、metadata、静态 quick-action projection 和 action 类型；即使逻辑已表格化，owner 仍在 1 万行以上的大 UI 文件里，后续继续迁移 formatter/动态 action 时缺少明确落点。
   - 结果：新增 `src/ui/monitor.rs`，迁移 `MonitorTier`、`MonitorTab`、`MonitorTabMetadata`、`MonitorQuickAction`、静态 quick-action template/table 与对应 projection 方法；`src/ui.rs` 通过 `mod monitor;` 使用这些类型，动态 action 和渲染路径保持不变；新增 `ui_module_docs_cover_monitor_projection_owner` 契约测试，要求 owner 文件存在、入口声明子模块、`docs/MODULES/ui.md` 记录该 owner。
   - 目的：继续推进 harness 阶段 6/7 的 UI 大文件瘦身和 projection ownership，为后续把 monitor formatter 与动态 quick actions 继续拆出建立明确模块边界。

187. Commands Action Checklist Projection 拆分
   - 产品缺口：`src/commands/productloop.rs` 除了 scorecard/round/benchmark 主流程外，还直接拥有 scorecard/local/benchmark checklist 过滤和命令 label 投影；这些 helper 被 doctor/env/session/config/git/prompt/skill/agent 等多个命令复用，不属于产品循环 handler 的主体职责，也让大命令模块继续膨胀。
   - 结果：新增 `src/commands/action_checklist.rs`，迁移 `scorecard_action_checklist`、`local_action_checklist`、`benchmark_action_checklist` 以及 scorecard/local/benchmark label 投影；`src/commands.rs` 统一 re-export 这些 helper，`productloop.rs` 的 round gate checklist 改为复用 `scorecard_action_checklist`；`commands_module_docs_cover_split_source_files` 增加该 owner 文件，`docs/MODULES/commands.md`、`docs/HARNESS.md` 和 `docs/ARCHITECTURE.md` 同步记录职责变化。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和投影 owner 收束，让 action/checklist 输出契约从产品循环主文件中独立出来；命令面 support/legacy 降级策略和 productloop/session/delivery 进一步瘦身仍需后续推进。

188. Legacy Command Successor Metadata
   - 产品缺口：命令面已经有 `legacy` 分组，但 registry 只记录 group/running-safe，`docs/COMMANDS.md` 也只写“stable alias”或兼容说明；后续 agent 无法从机器可读入口判断 legacy 命令应该导向哪个 successor，容易继续把 legacy 入口当成新功能中心。
   - 结果：新增 `LegacyCommandMetadata` 与 `CommandRouter::legacy_command_metadata()`，为 `/apikey`、`/compiler`、`/install`、`/cleanup`、`/rename` 记录 successor 和 policy；`docs/COMMANDS.md` 的 legacy 行内写明 `替代：...`；新增 `command_registry_owns_legacy_successor_metadata` 契约测试，确保每个 registry legacy 命令都有 successor/policy，且文档行包含对应替代入口。
   - 目的：继续推进 harness 阶段 6 的删除/降级策略，把 legacy 兼容从“只有分组标签”升级为可验证的迁移约束；真正删除或进一步降级命令仍需后续逐项评估。

189. UI Monitor SessionMonitor-only Formatter Owner
   - 产品缺口：`src/ui/monitor.rs` 已拥有 monitor tab catalog、metadata 与静态 quick-action projection，但只消费 `SessionMonitor` 的 Usage/Deliver/Tests/Session/Context/Environment/Approvals formatter 仍留在 `src/ui.rs`，导致已收束的核心 monitor projection 仍要回到 1 万行以上的 UI 入口文件里维护。
   - 结果：将 `format_usage_tab_lines`、`format_deliver_tab_lines`、`format_tests_tab_lines`、`format_session_tab_lines`、`format_context_tab_lines`、`format_environment_tab_lines`、`format_approvals_tab_lines` 与 quick-action 行追加 helper 迁入 `src/ui/monitor.rs`，`src/ui.rs` 只导入并调用这些 projection formatter；`ui_module_docs_cover_monitor_projection_owner` 契约测试新增 owner 断言，确保这些 formatter 不回流到 `src/ui.rs` 且 `docs/MODULES/ui.md` 同步记录。
   - 目的：继续推进 harness 阶段 6/7 的 UI 大文件瘦身和 projection ownership，让只消费会话观测模型的 tab 状态投影与 monitor metadata/quick actions 位于同一 owner；剩余需要 `TuiState`、workspace、配置或库状态的 formatter 与动态 quick actions 仍需后续继续迁移。

190. UI Monitor-only Dynamic Quick Actions Owner
   - 产品缺口：静态 quick actions 已经由 `src/ui/monitor.rs` 拥有，但 Tools/Deliver/Environment 这三个不需要 `TuiState` 的动态 quick actions 仍留在 `src/ui.rs`，让 monitor action 投影继续分散在入口文件和 owner 模块之间。
   - 结果：将 `tool_quick_actions`、`deliver_quick_actions`、`environment_quick_actions`、`environment_action_target`、`environment_needs_setup` 迁入 `src/ui/monitor.rs`，`src/ui.rs` 只在 tab 分发时调用这些 owner 函数；`ui_module_docs_cover_monitor_projection_owner` 契约测试新增 quick-action owner 断言，防止这些函数回流。
   - 目的：继续推进 harness 阶段 7 的 quick-action projection 收束，把不依赖 workspace/config/store 的 monitor actions 放到同一 owner；Health/Library 动态操作仍因读取 workspace、配置和本地库状态留在 `src/ui.rs`，后续可再评估是否抽成更具体的领域 projection。

191. UI Health Monitor Projection Owner
   - 产品缺口：Health tab formatter 与 quick actions 需要读取 workspace 和 `AppConfig`，不适合放进纯 `src/ui/monitor.rs`，但继续留在 `src/ui.rs` 会让入口文件承担 provider credential 投影、env key 归一化和 presence label 格式化等 tab 专属逻辑。
   - 结果：新增 `src/ui/monitor_health.rs`，迁移 `format_health_tab_lines`、`health_quick_actions_for_state`、`provider_needs_credentials_for_ui`、`provider_env_key_for_ui` 和 `presence_label`；`src/ui.rs` 只注册 `mod monitor_health;` 并调用 owner 函数；新增 `ui_module_docs_cover_monitor_health_owner` 契约测试，防止 Health projection 回流到入口文件。
   - 目的：继续推进 harness 阶段 6/7 的 UI 大文件瘦身和 projection ownership，把读取 workspace/config 的 Health 状态投影与纯 monitor projection 分离；Library/Trace/Result/Changes/Tools 等仍需后续按依赖继续拆分。

192. UI Library Monitor Projection Owner
   - 产品缺口：Library tab formatter 与 quick actions 需要读取 workspace、prompt store、skill store 和 agent store；这类本地库投影继续留在 `src/ui.rs` 会让入口文件承担 prompt/skill/agent inventory 格式化和按钮决策，不符合 UI projection owner 收束方向。
   - 结果：新增 `src/ui/monitor_library.rs`，迁移 `format_library_tab_lines`、`library_quick_actions_for_state` 和 `format_library_item`；`src/ui.rs` 只注册 `mod monitor_library;` 并调用 owner 函数；新增 `ui_module_docs_cover_monitor_library_owner` 契约测试，防止 Library projection 回流到入口文件。
   - 目的：继续推进 harness 阶段 6/7 的 UI 大文件瘦身和 projection ownership，把读取本地库状态的 Library 投影从入口文件移出；Trace/Result/Changes/Tools 等仍需后续按依赖继续拆分。

193. Benchmark Status Projection Owner
   - 产品缺口：`src/commands/productloop.rs` 同时承担 benchmark status 判定、freshness 计算、required preset 覆盖、JSON/text 格式化和其它 benchmark 子命令分发，阶段 6 要求旧大文件继续向明确 owner 收束。
   - 结果：新增 `src/commands/benchmark_status.rs`，迁移 `BenchmarkStatusReport`、required preset 状态、`build_benchmark_status_report`、`format_benchmark_status_json`、`format_benchmark_status_text`、freshness JSON/summary 和 benchmark artifact preset 匹配 helper；`src/commands.rs` 注册 `mod benchmark_status;` 并通过 crate 内 re-export 供 `/round`、`/scorecard`、`/benchmark status` 与 benchmark suite 输出复用；新增 `productloop_delegates_benchmark_status_projection` 契约测试防止该 projection 回流到 `productloop.rs`。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和状态投影 owner 收束；命令面真正删除/降级策略、`productloop.rs` 其它 benchmark 子域、`session.rs`、`delivery.rs` 和 UI 剩余 projection 仍需后续推进。

194. Benchmark Baselines Projection Owner
   - 产品缺口：SOTA baseline 下一步动作、baseline-template、baseline inventory 和 compare-ready 判断仍散落在 `src/commands/productloop.rs` 的文件开头、handler、JSON/text formatter 与 helper 区域，导致产品循环主文件继续承担 baseline projection owner 职责。
   - 结果：新增 `src/commands/benchmark_baselines.rs`，迁移 `sota_baseline_next_actions`、默认 baseline action 常量、`handle_benchmark_baselines`、`handle_benchmark_baseline_template`、baseline report/case 类型、baseline 文件加载、baseline-template JSON/text、baseline inventory JSON/text 和 compare-ready 判断；`src/commands.rs` 注册 `mod benchmark_baselines;` 并通过 crate 内 re-export 供 `/round`、`/scorecard`、`/recipes`、`/opportunities`、benchmark compare/trends 复用；新增 `productloop_delegates_benchmark_baselines_projection` 契约测试防止 baseline projection 回流到 `productloop.rs`。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和 baseline projection owner 收束；当时 `productloop.rs` 仍保留 benchmark run/record/list/show/cleanup/summary/trends/compare 与 scorecard/round 逻辑，后续继续拆分或降级评估。

195. Benchmark History Projection Owner
   - 产品缺口：`src/commands/productloop.rs` 仍同时承担 benchmark summary/trends/compare 的 handler、历史聚合、趋势计算、baseline comparison、JSON/text projection 和 `/round`/`/scorecard` trend gate 状态判断，导致产品循环主文件继续承载 benchmark history 子域。
   - 结果：新增 `src/commands/benchmark_history.rs`，迁移 `handle_benchmark_summary`、`handle_benchmark_trends`、`handle_benchmark_compare`、`build_benchmark_case_summaries`、`build_benchmark_case_trends`、summary/trends/compare JSON/text formatter、case summary/trend/comparison 类型与 trend gate 状态判断；`src/commands.rs` 注册 `mod benchmark_history;` 并通过 crate 内 re-export 供 `/benchmark summary|trends|compare`、`/round` 与 `/scorecard` 复用；新增 `productloop_delegates_benchmark_history_projection` 契约测试防止 history projection 回流到 `productloop.rs`。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和 benchmark history projection owner 收束；当时 `productloop.rs` 仍保留 benchmark run/record/list/show/cleanup 与 scorecard/round 逻辑，后续继续拆分或降级评估。

196. Benchmark Artifact Projection Owner
   - 产品缺口：`src/commands/productloop.rs` 仍同时承担 benchmark list/show/cleanup handler、artifact 读取排序、artifact detail summary、cleanup candidate 选择、artifact JSON/text projection 和 `BENCHMARK_ARTIFACT_SCHEMA` 所有权，导致 benchmark evidence artifact 生命周期没有独立 owner。
   - 结果：新增 `src/commands/benchmark_artifacts.rs`，迁移 `BENCHMARK_ARTIFACT_SCHEMA`、`BenchmarkArtifact`、`handle_benchmark_list`、`handle_benchmark_show`、`handle_benchmark_cleanup`、`load_benchmark_artifacts`、artifact status/duration/string helper、artifact detail/list/cleanup JSON/text projection 和 cleanup 删除预览逻辑；`src/commands.rs` 注册 `mod benchmark_artifacts;` 并通过 crate 内 re-export 供 benchmark run/record、status、history、baseline 与 `/benchmark list|show|cleanup` 复用；新增 `productloop_delegates_benchmark_artifact_projection` 契约测试防止 artifact projection 回流到 `productloop.rs`。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和 artifact projection owner 收束；当时 `productloop.rs` 仍保留 benchmark run/record/run-suite/presets 与 scorecard/round 逻辑，后续继续拆分或降级评估。

197. Benchmark Presets Catalog Owner
   - 产品缺口：`src/commands/productloop.rs` 仍直接拥有 `BenchmarkPreset`、`BENCHMARK_PRESETS`、required evidence preset 清单、default suite preset 清单、preset resolver 和 `/benchmark presets` JSON/text projection，导致 benchmark run-suite、status、baseline 等模块共享的 preset catalog 没有独立 owner。
   - 结果：新增 `src/commands/benchmark_presets.rs`，迁移 `BenchmarkPreset`、`BENCHMARK_PRESETS`、`MEANINGFUL_BENCHMARK_PRESETS`、`DEFAULT_BENCHMARK_RUN_SUITE_PRESETS`、`benchmark_preset_by_name`、`handle_benchmark_presets`、presets JSON/text formatter 与 preset summary projection；`src/commands.rs` 注册 `mod benchmark_presets;` 并通过 crate 内 re-export 供 benchmark run-suite、status、baseline、tests 和 `/benchmark presets` 复用；新增 `productloop_delegates_benchmark_presets_catalog` 契约测试防止 preset catalog 回流到 `productloop.rs`。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和 preset catalog owner 收束；`productloop.rs` 仍保留 benchmark run/record/run-suite 与 scorecard/round 逻辑，后续仍需继续拆分或降级评估。

198. Benchmark Runs Execution Owner
   - 产品缺口：`src/commands/productloop.rs` 仍直接拥有 benchmark run、benchmark record、benchmark run-suite 的 option 解析、shell 执行、timeout、execution artifact JSON、suite schema、artifact path slug 和 run-suite 输出 projection，导致产品循环主文件继续承担 benchmark 执行子域。
   - 结果：新增 `src/commands/benchmark_runs.rs`，迁移 `BENCHMARK_RUN_SUITE_REMEDIATION_ACTION`、`BENCHMARK_SUITE_SCHEMA`、`BenchmarkRunSuiteOptions`、`BenchmarkRunArtifact`、`BenchmarkCommandExecution`、`handle_benchmark_run`、`handle_benchmark_record`、`handle_benchmark_run_suite`、`execute_benchmark_run_artifact`、benchmark run/record JSON builder、shell command execution、output truncation、artifact path 生成与 `benchmark_slug`；`src/commands.rs` 注册 `mod benchmark_runs;` 并通过 crate 内 re-export 供 `/benchmark run|record|run-suite`、`/round --run-benchmark`、status/artifact/baseline/tests 复用；新增 `productloop_delegates_benchmark_runs_execution` 契约测试防止执行子域回流到 `productloop.rs`。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身和 benchmark execution artifact owner 收束；`productloop.rs` 现在主要保留 scorecard/round 构建、benchmark 分发与 gate 汇总，后续仍需继续复查 support/legacy 策略、`session.rs`、`delivery.rs` 和 UI 剩余 projection。

199. Benchmark Status Handler Owner
   - 产品缺口：status projection 已拆出后，`src/commands/productloop.rs` 仍保留 `BENCHMARK_STATUS_SCHEMA`、freshness 阈值、`BenchmarkStatusOptions`、`handle_benchmark_status` 与 status option parser，导致 `/benchmark status|gate` 的 handler ownership 仍不完整。
   - 结果：将 status schema/freshness 常量、status options、`handle_benchmark_status` 和 `parse_benchmark_status_options` 迁入 `src/commands/benchmark_status.rs`；`productloop.rs` 的 `/benchmark status|gate` 分发继续通过 crate 内 re-export 调用该 owner；`productloop_delegates_benchmark_status_projection` 契约测试扩展为同时约束 status handler 和常量不回流。
   - 目的：补齐 benchmark status owner 边界，让 `productloop.rs` 不再直接拥有 status 子命令执行细节；后续继续处理 scorecard/round 主体、`session.rs`、`delivery.rs` 与 UI 剩余 projection。

200. Scorecard Opportunity Projection Owner
   - 产品缺口：`src/commands/productloop.rs` 仍直接拥有 `ScorecardOpportunity`、scorecard 产品机会生成、baseline nextActions 合并、opportunity JSON/text projection、recommended opportunity 以及 priority/effort counts；这些输出被 `/scorecard`、`/round`、`/recipes` 和 `/opportunities` 共同复用，不应继续由产品循环主文件承担 projection owner。
   - 结果：新增 `src/commands/scorecard_opportunities.rs`，迁移 `ScorecardOpportunity`、`SCORECARD_ROUND_REPORT_ACTION`、`SCORECARD_OPPORTUNITIES_ACTION`、`scorecard_product_opportunities`、`opportunity_baseline_next_actions`、opportunity JSON/text projection、recommended opportunity projection 以及 priority/effort counts；`src/commands.rs` 注册并重新导出该 owner，`productloop.rs` 只消费这些 projection helper；新增 `productloop_delegates_scorecard_opportunity_projection` 契约测试防止 opportunity projection 回流。
   - 目的：继续推进 harness 阶段 6 的产品循环大模块瘦身，让 scorecard opportunity 输出契约有独立 owner；`productloop.rs` 仍保留 scorecard/round 报告构建、benchmark 分发与 gate 汇总，后续仍需继续复查 support/legacy 策略、`session.rs`、`delivery.rs` 和 UI 剩余 projection。

201. Benchmark Dispatch Owner
   - 产品缺口：benchmark status/run/history/artifact/preset/baseline 子域已经拆出后，`src/commands/productloop.rs` 仍直接拥有 `/benchmark` 子命令分发表、scorecard-compatible benchmark args 判断和 benchmark gate dispatch 默认失败参数注入，导致产品循环主文件继续承担 benchmark CLI 路由职责。
   - 结果：新增 `src/commands/benchmark_dispatch.rs`，迁移 `handle_benchmark`、`benchmark_status_args_request_failure` 和 `benchmark_args_are_scorecard_compatible`；`src/commands.rs` 注册并重新导出 `handle_benchmark`，`productloop.rs` 不再直接拥有 benchmark 子命令分发；新增 `productloop_delegates_benchmark_dispatch` 契约测试防止该分发表回流。
   - 目的：继续推进 harness 阶段 6 的产品循环大模块瘦身，让 `/benchmark` CLI 路由只负责委派到各 benchmark owner；`productloop.rs` 现在更聚焦于 `/scorecard` 与 `/round` 报告构建，后续仍需继续拆分 scorecard/round builder、`session.rs`、`delivery.rs` 和 UI 剩余 projection。

202. Round Benchmark Gate Projection Owner
   - 产品缺口：`src/commands/productloop.rs` 仍直接拥有 round benchmark trend gate 判定、benchmark gate summary、round benchmark status JSON 和 freshness suffix；这些 helper 被 scorecard nextActions、round gates、round JSON/text 和 benchmark suite 输出共同复用，不属于 scorecard/round 报告构建主体。
   - 结果：新增 `src/commands/round_benchmark_gates.rs`，迁移 `round_benchmark_trends_needs_attention`、trend gate summary/gap/action、`round_benchmark_gate_summary`、round benchmark status projection 和 freshness suffix；`src/commands.rs` 注册并重新导出该 owner，`benchmark_runs.rs` 改为通过命令层 re-export 复用 `round_benchmark_status_json`；新增 `productloop_delegates_round_benchmark_gate_projection` 契约测试防止 gate/status projection 回流。
   - 目的：继续推进 harness 阶段 6 的产品循环大模块瘦身，把 benchmark gate/status 投影从报告构建主文件移出；`productloop.rs` 现在主要剩 scorecard/round builder、round goal status 与文本/JSON 输出，后续可继续沿这些边界拆分。

203. Round Goal Status Projection Owner
   - 产品缺口：`src/commands/productloop.rs` 仍直接拥有 `RoundGoalStatus`、goal readiness 会话选择和 `goalStatus` JSON projection；这些逻辑读取 session/goal readiness，不属于 round 报告拼装本体，且与 goal 模块边界耦合较强。
   - 结果：新增 `src/commands/round_goal_status.rs`，迁移 `RoundGoalStatus`、`build_round_goal_status` 和 `round_goal_status_json`；`src/commands.rs` 注册并重新导出该 owner，`productloop.rs` 只消费 round goal status projection 结果来构建 gate 和文本输出；新增 `productloop_delegates_round_goal_status_projection` 契约测试防止该 projection 回流。
   - 目的：继续推进 harness 阶段 6 的产品循环大模块瘦身，把 goal readiness projection 与 round report builder 分离；`productloop.rs` 现在主要剩 scorecard/round builder、round benchmark suite wrapper 与文本/JSON 输出，后续继续按这些边界拆分。

204. Scorecard Report Builder Owner
   - 产品缺口：`src/commands/productloop.rs` 仍直接拥有 `/scorecard` handler、scorecard category projection、scorecard report builder、scorecard text/JSON output 和 scorecard summary JSON；这些逻辑已经形成独立报告域，继续留在 product loop 主文件会让 `/round` 编排和 `/scorecard` 报告构建混在一起。
   - 结果：新增 `src/commands/scorecard_report.rs`，迁移 `ScorecardReport`、`ScorecardCategory`、`SCORECARD_BENCHMARK_REMEDIATION_ACTION`、`handle_scorecard`、scorecard option parsing、`build_scorecard_report`、category scoring helpers、scorecard text/JSON formatter 和 `scorecard_summary_json`；`src/commands.rs` 注册并重新导出该 owner，`benchmark_runs.rs` 和 `productloop.rs` 通过命令层 re-export 复用 scorecard report projection；新增 `productloop_delegates_scorecard_report_builder` 契约测试防止 scorecard report builder 回流。
   - 目的：继续推进 harness 阶段 6 的产品循环大模块瘦身，把 `/scorecard` 报告构建与 `/round` 编排分离；`productloop.rs` 现在主要剩 `/round` handler、round builder、round benchmark suite wrapper 和 round text/JSON output，后续可继续拆 round report owner。

205. Round Report Builder Owner
   - 产品缺口：scorecard/benchmark/goal 子域拆出后，`src/commands/productloop.rs` 仍直接拥有 `/round` handler、round report builder、round text/JSON output、round summary JSON 和 `/round --run-benchmark` benchmark suite wrapper，旧产品循环大文件仍承担主体输出契约。
   - 结果：新增 `src/commands/round_report.rs`，迁移 `DEFAULT_ROUND_SCORE_THRESHOLD`、`RoundReport`、`RoundGate`、`RoundBenchmarkRun`、`RoundTextInput`、`handle_round`、round option parsing、round benchmark suite wrapper、`build_round_report`、round text/JSON formatter、round summary/checklist JSON 与 benchmark run JSON；`src/commands/productloop.rs` 退化为兼容 re-export，`src/commands.rs` 注册新 owner，内部 round formatter 测试改为直接引用 `round_report`；新增 `productloop_delegates_round_report_builder` 契约测试防止 round report builder 回流。
   - 目的：完成产品循环大模块中 scorecard/round 主体报告 owner 的分离，让 `productloop.rs` 不再拥有业务实现；后续阶段 6/7 继续复查低价值/重复命令、`session.rs`、`delivery.rs` 和 UI projection model。

206. Delivery Diff Projection Owner
   - 产品缺口：`src/commands/delivery.rs` 同时承担 `/diff` 参数解析、path scope filtering、session diff fallback、diff stat/name-only/display projection 和 review/verify/handoff 共享的 diff path classifier；这些纯 diff/source helper 和 delivery 报告编排混在一起，让 3000+ 行的大命令模块继续膨胀。
   - 结果：新增 `src/commands/delivery_diff.rs`，迁移 `DiffOptions`、`DiffView`、`SESSION_DIFF_FALLBACK_LIMIT`、`SessionDiffSource`、`parse_diff_args`、`parse_review_args`、scope path 校验、`filter_diff_by_paths`、diff display/stat/name-only projection、session diff source/fallback、`session_diff_review_input`、`is_added_diff_line` 与 `review_path_from_diff_line`；`src/commands.rs` 注册并重新导出该 owner，delivery 单测改为直接引用 `delivery_diff`；新增 `delivery_delegates_diff_projection_owner` 契约测试防止 diff projection 回流。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身，把 diff 输入与 fallback 契约从 delivery 报告编排中拆出；后续可继续拆 verify/handoff report builder、`session.rs` 和 UI projection model。

207. UI Monitor Output Projection Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 Result/Trace advanced tab formatter 和 Result 输出窗口大小计算；这些只读 output projection 与 TUI 输入、鼠标、布局、运行安全分发混在 9000+ 行入口文件里，不符合阶段 7 的 projection model 收束方向。
   - 结果：新增 `src/ui/monitor_output.rs`，迁移 `format_result_tab_lines`、`result_output_window_size` 和 `format_trace_tab_lines`；`src/ui.rs` 注册 `mod monitor_output;` 并只调用 owner formatter；新增 `ui_module_docs_cover_monitor_output_owner` 契约测试防止 Result/Trace output projection 回流。
   - 目的：继续推进 harness 阶段 7 的 UI projection ownership，把 advanced output formatter 从 UI 入口中拆出；后续可继续评估 Changes/Tools 这类带交互状态的 projection 是否可拆成更细 owner。

208. Session Restore-Backup Owner
   - 产品缺口：`src/commands/session.rs` 同时承担 `/session` 大分发、restore-backup 参数解析、dry-run 预览、写回执行、备份选择、恢复目标解析、preview diff、JSON/text 报告和 UI running-safe dry-run 入口，导致 session 大模块继续混入独立恢复子域。
   - 结果：新增 `src/commands/session_restore.rs`，迁移 `handle_restore_backup`、`handle_restore_backup_dry_run`、restore-backup parser、dry-run renderer、backup/session/target resolver、preview diff、nextActions、text report 和 `deepcli.session.restore_backup.v1` JSON formatter；`src/commands.rs` 注册并重新导出该 owner，`session.rs` 只保留 `/session restore-backup|restore` 分发调用；将 `session_backup_record_json` 与 `session_matches_fallback_kind` 提升为 crate 内可见复用；新增 `session_delegates_restore_backup_owner` 契约测试防止恢复子域回流。
   - 目的：继续推进 harness 阶段 6 的大命令模块瘦身，把 session backup restore 从 session 主分发/检查/诊断投影中拆出；后续可继续拆 session list/search/inspect/report projection 或 delivery verify/handoff report builder。

209. Session Catalog Owner
   - 产品缺口：`src/commands/session.rs` 仍直接拥有默认 session 列表、`/session list`、`/session search`、`/session prune-empty` 的参数解析、catalog 查询、空会话清理、JSON/text projection、nextActions 和 checklist 生成，导致 session 主分发继续承担 catalog/report 子域。
   - 结果：新增 `src/commands/session_catalog.rs`，迁移 `handle_session_default_list`、`handle_session_list`、`handle_session_search`、`handle_session_prune_empty`、list/search/prune-empty parser、`SessionListReport`、`SessionSearchReport`、`SessionPruneEmptyReport`、catalog JSON/text projection、search matching、session list item JSON、空会话过滤与删除逻辑；`src/commands.rs` 注册并重新导出该 owner，`session.rs` 只保留 list/search/prune-empty 的分发调用；新增 `session_delegates_catalog_owner` 契约测试防止 catalog 子域回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把 catalog/list/search/prune-empty 输出契约从 session 检查、诊断、审批/旁路问题队列中拆出；后续可继续拆 session inspect/history/tools/tests/diffs/backups projection 或 delivery verify/handoff report builder。

210. Delivery Report Builder Owner
   - 产品缺口：`src/commands/delivery.rs` 在 diff projection 拆出后仍直接拥有 `/verify` 与 `/handoff` 的报告输入类型、验证/交接报告拼装、blocker/nextActions/checklist 投影、环境证据格式化和 Markdown/PR/JSON 输出，导致命令编排、执行副作用与报告契约继续混在同一大文件里。
   - 结果：新增 `src/commands/delivery_reports.rs`，迁移 `VerificationDiffSource`、`VerificationTestRun`、`VerificationEnvironmentCheck`、`VerificationStatusSource`、`VerificationReportInput`、`HandoffReportInput`、`format_verification_report`、`format_handoff_report`、verification/handoff JSON formatter、handoff Markdown/PR formatter、blocker extraction、delivery action checklist、环境证据 JSON/text projection、强/弱测试证据判定和 stale test evidence blocker；`src/commands.rs` 注册并重新导出该 owner；当时 `delivery.rs` 只保留 diff/review/verify/handoff 的命令编排、选项解析、test/env 执行和 worktree review heuristic，后续 review heuristic 已继续拆出；新增 `delivery_delegates_report_builder_owner` 契约测试防止报告 builder 回流。
   - 目的：继续推进 harness 阶段 6 的 delivery 大模块瘦身，把 verification report projection、handoff report projection 与 delivery report JSON 从命令编排中分离；后续已继续拆出 delivery review heuristic，仍需继续复查 `session.rs`、`delivery.rs` 剩余编排和 UI 剩余 projection。

211. Session Inspect Owner
   - 产品缺口：`src/commands/session.rs` 在 catalog 与 restore 拆出后仍直接拥有 `/session show|history|summary|tools|tests|diffs|backups` 的子命令 handler、record inspect parser、工具/测试/diff/backup 文本格式化、inspect JSON、记录 JSON projection 和失败工具筛选；这些只读 record projection 与 next/diagnose/rename/export 编排混在同一大文件里。
   - 结果：新增 `src/commands/session_inspect.rs`，迁移 `handle_session_show`、`handle_session_history`、`handle_session_summary`、`handle_session_tools`、`handle_session_tests`、`handle_session_diffs`、`handle_session_backups`、`SessionInspectOptions`、`ToolCallFilter`、record inspect parser、session inspect JSON、session record projection、工具/测试/diff/backup record JSON、失败工具筛选、`format_tool_calls`、`format_test_runs`、`format_session_diffs` 和 `format_session_backups`；`src/commands.rs` 注册并重新导出该 owner，`session.rs` 只保留对应子命令分发调用；新增 `session_delegates_inspect_owner` 契约测试防止 inspect projection 回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把 session inspect JSON 与 session tools/tests/diffs/backups projection 从 session next/diagnose 和导出编排中分离；后续可继续拆 session next/diagnose projection 或复查 support/legacy 命令面。

212. Session Recovery Owner
   - 产品缺口：`src/commands/session.rs` 在 inspect 拆出后仍直接拥有 `/session next` 与 `/session diagnose` 的子命令 handler、next/diagnose 参数解析、恢复候选选择、next-action signals、quick links、诊断报告和 `deepcli.session.next.v1`/`deepcli.session.diagnose.v1` JSON projection，导致恢复信号、诊断输出与 session 主分发继续耦合。
   - 结果：新增 `src/commands/session_recovery.rs`，迁移 `handle_session_next`、`handle_session_diagnose`、`resolve_session_for_next_actions`、`session_has_next_action_signals`、next/diagnose parser、`format_session_next_actions`、`format_session_next_json`、`format_session_diagnosis`、`format_session_diagnosis_json`、next action/quick link projection、session next signals JSON 和诊断 tool/test/plan JSON projection；`src/commands.rs` 注册并重新导出该 owner，`session.rs` 只保留 `/session next|diagnose` 分发调用；新增 `session_delegates_recovery_owner` 契约测试防止 recovery projection 回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把 session next projection、session diagnose projection 与 next-action signals 从 session 主文件拆出；后续已继续拆出 session export、rename 与 resumable owner，仍可继续评估 running-safe/session 主分发是否足够薄，或转向 UI projection owner。

213. UI Monitor Changes Projection Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 Changes tab 的 worktree snapshot、Git status/diff 解析、session diff 聚合、patch formatter、键盘滚动和鼠标命中逻辑；这些带交互状态的 projection 与 TUI 输入、布局和 running-safe 分发混在同一入口文件里，不符合阶段 7 的 projection model 收束方向。
   - 结果：新增 `src/ui/monitor_changes.rs`，迁移 `WorkspaceChangesSnapshot`、`WorkspaceDiffSection`、`handle_changes_tab_key`、`select_change_patch_at_row`、`refresh_workspace_changes_snapshot`、`format_changes_tab_lines`、`append_workspace_changes_lines`、`load_workspace_changes_snapshot`、`parse_git_status_snapshot`、`parse_diff_sections` 以及 diff preview/session diff summary helper；`src/ui.rs` 注册 `mod monitor_changes;` 并只保留状态字段和调用；新增 `ui_module_docs_cover_monitor_changes_owner` 契约测试防止 Changes projection 回流。
   - 目的：继续推进 harness 阶段 6/7 的 UI 大文件瘦身和 projection ownership，把 Changes tab 的 workspace/session diff projection 从 UI 入口中拆出；后续可继续评估 Tools tab 的工具详情/选择/动作 projection 是否需要独立 owner。

214. UI Monitor Tools Projection Owner
   - 产品缺口：`src/ui.rs` 在 Changes projection 拆出后仍直接拥有 Tools tab 的 `ToolLogItem`/`ToolTabLine`、工具详情预览/截断、选择/展开、可见行映射、Ctrl-O/Ctrl-F 预填和工具 tab formatter；这些带交互状态的 projection 不应继续和 TUI 主循环、running-safe 分发及布局代码混在同一入口文件里。
   - 结果：新增 `src/ui/monitor_tools.rs`，迁移 `ToolLogItem`、`ToolTabLine`、`handle_tools_tab_key`、`prefill_tools_session_command`、`toggle_selected_tool`、`toggle_tool_at_row`、`move_selected_tool_by`、`select_tool_at_index`、`visible_tool_index_at_line`、`selected_tool_panel_line`、`format_tool_tab_lines`、`tool_tab_lines`、`append_tool_quick_action_lines`、`tool_detail_preview_lines` 和 `tool_detail_is_truncated`；`src/ui.rs` 注册 `mod monitor_tools;` 并只保留工具列表状态、外层鼠标滚动分发和调用；新增 `ui_module_docs_cover_monitor_tools_owner` 契约测试防止 Tools projection 回流。
   - 目的：继续推进 harness 阶段 6/7 的 UI 大文件瘦身和 projection ownership，把 Tools tab 的工具日志/详情 projection 从 UI 入口中拆出；后续阶段 7 主要剩真实终端观感验收与 UI 入口剩余职责复查。

215. Delivery Review Heuristic Owner
   - 产品缺口：`src/commands/delivery.rs` 在 diff projection 与 report builder 拆出后仍直接拥有 `review_diff`、`review_worktree` 以及 sensitive/dangerous/panic-prone 风险检测；这些信号被 `/review`、`/verify`、`/handoff` 共同复用，不应继续和交付命令编排、test/env 执行混在同一文件里。
   - 结果：新增 `src/commands/delivery_review.rs`，迁移 `ReviewFindings`、`ReviewFinding`、`review_diff`、`review_worktree`、凭据路径识别、测试/文档路径豁免、敏感行检测、危险命令检测、panic-prone 检测、detector literal/source 过滤和 finding example 投影；`src/commands.rs` 注册并重新导出该 owner，`delivery.rs` 只保留交付命令编排和 owner 委派；新增 `delivery_delegates_review_heuristic_owner` 契约测试防止 review heuristic 回流。
   - 目的：继续推进 harness 阶段 6 的 delivery 大模块瘦身，把 review risk detection 与 sensitive/dangerous/panic-prone finding projection 从命令编排中分离；后续继续复查 support/legacy 降级策略、`session.rs` 剩余主分发/running-safe 职责和 UI 入口剩余职责。

216. Session Export Owner
   - 产品缺口：`src/commands/session.rs` 在 catalog、restore、inspect、recovery 拆出后仍直接拥有 `/session export` 的参数解析、session id/当前会话选择、export path safety、默认 artifact 路径和 session export JSON 写出；这些文件写入与路径安全逻辑不应继续和 `/session` 主分发、rename、可恢复会话筛选混在同一文件里。
   - 结果：新增 `src/commands/session_export.rs`，迁移 `handle_session_export`、`parse_export_args`、`resolve_export_path` 和 `export_session`；`src/commands.rs` 注册该 owner 并重新导出 `handle_session_export`，`session.rs` 的 `/session export` 分支只保留 owner 委派；命令测试改为直接引用 `session_export::parse_export_args`；新增 `session_delegates_export_owner` 契约测试防止 export parser/path safety/JSON 写出回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把 session export parser、export path safety 与 session export JSON 从主分发中分离；后续已继续拆出 session rename owner，仍需评估可恢复会话筛选、support/legacy 降级策略和 UI 入口剩余职责。

217. Session Rename Owner
   - 产品缺口：`src/commands/session.rs` 在 export 拆出后仍直接拥有 `/session rename` 的参数解析、`--current` 解析、空标题校验、session title update 和文本回执；这些标题更新逻辑不应继续和 `/session` 主分发、running-safe 处理与可恢复会话筛选混在同一文件里。
   - 结果：新增 `src/commands/session_rename.rs`，迁移 `handle_session_rename` 和 `parse_session_rename_args`；`src/commands.rs` 注册该 owner 并重新导出 `handle_session_rename`，`session.rs` 的 `/session rename` 分支只保留 owner 委派；新增 `session_delegates_rename_owner` 契约测试防止 rename parser/title update 回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把 session rename parser、current-session rename 与 session title update 从主分发中分离；后续已继续拆出可恢复会话筛选，仍需复查 support/legacy 降级策略、delivery 剩余编排、session running-safe/主分发和 UI 入口剩余职责。

218. Session Resumable Owner
   - 产品缺口：`src/commands/session.rs` 在 export/rename 拆出后仍直接拥有当前 workspace 可恢复候选筛选、低信息 clarification 过滤、thin completed chat 过滤、可恢复候选列表文本和无显式 id 时的 workspace fallback；这些逻辑被 `/resume`、`/fork`、selftest 和 TUI resume picker 共同复用，不应继续和 `/session` 主分发、inspection fallback 混在同一文件里。
   - 结果：新增 `src/commands/session_resumable.rs`，迁移 `format_resumable_session_list`、`sessions_with_resumable_context`、`filter_session_metadata_with_resumable_context`、`session_metadata_matches_workspace`、`session_has_resumable_context`、low-information clarification 过滤、thin completed chat 过滤、metric footer stripping、`format_limited_resumable_session_list` 和 `resolve_resumable_session_for_workspace`；`src/commands.rs` 注册并重新导出该 owner，保留 `SessionFallbackKind` 与通用 inspection fallback 在 `session.rs`；新增 `session_delegates_resumable_owner` 契约测试防止可恢复筛选回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把 resumable session filtering、low-information clarification filter、thin completed chat filter 与 workspace resumable fallback 从主分发中分离；后续继续复查 support/legacy 降级策略、delivery 剩余编排、session running-safe/主分发和 UI 入口剩余职责。

219. UI Running Command Owner
   - 产品缺口：`src/ui.rs` 在 monitor projection 多轮拆出后仍直接拥有运行中本地命令解析、read-only/write-output guard、状态/BTW/Terminal/Git 旁路处理、unsupported/deferred 提示和命令执行结果写回；这让 UI 入口继续承担 command safety gate 与旁路命令编排，不符合阶段 7 “UI 消费 projection、入口只保留外层 TUI 编排”的收束方向。
   - 结果：新增 `src/ui/running_commands.rs`，迁移 `handle_running_tui_local_command`、`running_tui_supported_command_hint`、`running_tui_deferred_input_hint`、`ensure_running_no_output`、`ensure_running_completion_is_observation_only`、`ensure_running_round_is_read_only`、`ensure_running_benchmark_is_read_only`、`ensure_running_preflight_is_planned`、`ensure_running_session_is_read_only`、`ensure_running_git_is_read_only`、`handle_tui_running_git`、`format_tui_running_status`、`handle_tui_running_btw`、`handle_tui_running_terminal` 和运行中 BTW 列表 formatter；`src/ui.rs` 注册该 owner 并只保留 TUI 状态、主事件循环、`/stop` 后的 worker abort、session pause 和 runtime rebuild；新增 `ui_module_docs_cover_running_command_owner` 契约测试防止 running command owner 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把运行中命令 safety gate 和旁路处理从 UI 主文件中分离；后续继续复查 `src/ui.rs` 是否还存在可拆出的 resume picker、审批交互或布局/渲染 owner，并补真实终端观感验收。

220. UI Resume Picker Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 resume picker 状态、metadata 过滤、独立全屏选择循环、TUI 内键鼠处理、列表/预览布局和会话预览文本；这些会话选择 projection 与主 TUI 事件循环、运行中命令、monitor 渲染混在一起，不符合阶段 7 的 UI owner 收束方向。
   - 结果：新增 `src/ui/resume_picker.rs`，迁移 `ResumePicker`、`ResumeSelection`、`pick_resume_session`、`session_matches_resume_query`、`run_resume_picker_loop`、`resume_filter_accepts_char`、`handle_resume_picker_key`、`handle_resume_picker_mouse_for_state`、`handle_resume_picker_mouse`、`resume_picker_layout`、`render_resume_picker` 和 `format_resume_preview_text`；`src/ui.rs` 注册并复用该 owner，保留当前 picker 状态字段和恢复结果应用；新增 `ui_module_docs_cover_resume_picker_owner` 契约测试防止 resume picker 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 resume picker 的筛选、选择、预览和渲染从 UI 主文件中拆出；后续继续复查审批/旁路问题交互、命令 palette、布局/渲染等剩余入口职责。

221. UI Approval Interaction Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 Approvals tab 的 pending approval / open BTW blocker 选择、鼠标命中、批准/拒绝、BTW 回答 prompt、session/runtime 写回和选择 clamp；这些交互规则与 TUI 主循环、monitor 渲染和输入框处理混在一起，不符合阶段 7 的 owner 收束方向。
   - 结果：新增 `src/ui/approvals.rs`，迁移 `SideQuestionPrompt`、`SelectedBlocker`、`handle_approval_tab_key`、`handle_approvals_mouse_for_state`、`clicked_approvals_tab_index`、`selected_blocker`、`activate_selected_blocker`、`deny_selected_blocker`、`open_side_question_answer_prompt`、`handle_side_question_prompt_key`、`confirm_side_question_prompt`、`answer_side_question_for_state`、`update_selected_approval`、`update_approval_for_state` 和 `clamp_selected_blocker_to_monitor`；`src/ui.rs` 注册并复用该 owner，只保留选择状态字段、prompt 状态字段和输入框渲染；新增 `ui_module_docs_cover_approval_interaction_owner` 契约测试防止 approval interaction 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把审批/旁路问题交互与写回逻辑从 UI 主文件中拆出；后续继续复查命令 palette、credential prompt、布局/渲染等剩余入口职责。

222. UI Command Palette Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 slash command palette 的命令查询、running-safe 排序、匹配上限、键鼠选择、点击命中、完成输入和渲染；这些命令发现/交互规则与主 TUI 事件循环、monitor 渲染和运行中命令分发混在一起，不符合阶段 7 的 owner 收束方向。
   - 结果：新增 `src/ui/command_palette.rs`，迁移 `COMMAND_PALETTE_MATCH_LIMIT`、`RUNNING_SAFE_PALETTE_PRIORITY`、`handle_command_palette_key`、`handle_command_palette_mouse_for_state`、`clicked_command_palette_index`、`command_palette_selection_event`、`complete_selected_command`、`clamp_selected_command`、`slash_command_suggestions_for_state`、`prioritize_running_safe_suggestions`、`running_safe_palette_priority`、`slash_command_query`、`render_command_palette`、`format_command_palette_text`、`command_palette_match_token` 和 `command_palette_matches_line_index`；`src/ui.rs` 注册并复用该 owner，只保留当前选中命令索引和外层事件/渲染分发；新增 `ui_module_docs_cover_command_palette_owner` 契约测试防止 command palette 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 slash palette 查询、排序、完成和渲染从 UI 主文件中拆出；后续继续复查 credential prompt、布局/渲染等剩余入口职责，并补真实终端观感验收。

223. UI Credential Prompt Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 `/credentials set` 解析、隐藏输入框打开、prompt 按键处理、API key 保存、隐藏正文和隐藏光标计算；这些 credentials 交互规则与主 TUI 输入循环和渲染路径混在一起，不符合阶段 7 的 owner 收束方向。
   - 结果：新增 `src/ui/credential_prompt.rs`，迁移 `CredentialPrompt`、`CredentialPromptSpec`、`handle_tui_credential_set_for_state`、`handle_credential_prompt_key`、`confirm_credential_prompt`、`parse_tui_credential_set`、`credential_prompt_hidden_body` 和 `credential_prompt_hidden_cursor`；`src/ui.rs` 注册并复用该 owner，只保留当前 credential prompt 状态和外层输入/渲染分发；新增 `ui_module_docs_cover_credential_prompt_owner` 契约测试防止 credential prompt 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 credential prompt 的解析、隐藏输入和写回逻辑从 UI 主文件中拆出；后续继续复查布局/渲染等剩余入口职责，并补真实终端观感验收。

224. UI Chat View Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有主聊天布局、transcript 窗口计算、消息正文拼接、整体 chat UI 渲染和 message-box 光标定位；这些布局/渲染职责与 TUI 事件循环、monitor owner 和运行中命令处理混在一起，不符合阶段 7 的 owner 收束方向。
   - 结果：新增 `src/ui/chat_view.rs`，迁移 `ChatUiLayout`、`chat_ui_layout`、`transcript_visible_message_count`、`transcript_window`、`format_transcript_text`、`format_messages_title`、`render_chat_ui` 和 `message_box_cursor_position`；`src/ui.rs` 注册并复用该 owner，只保留外层 TUI 事件循环、状态字段、task monitor owner 和运行中 worker 编排；新增 `ui_module_docs_cover_chat_view_owner` 契约测试防止 chat view 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把主聊天布局、消息渲染和输入光标定位从 UI 主文件中拆出；后续需要复查入口是否只剩外层编排，并补真实终端观感验收。

225. Session Selection Owner
   - 产品缺口：`src/commands/session.rs` 在 catalog/restore/inspect/recovery/export/rename/resumable 拆出后仍直接拥有 `SessionFallbackKind`、metadata JSON、inspection fallback、scoped list/action parser、queue action parser、approval/BTW cross-session lookup、session note prefix 和 `short_id` 投影；这些逻辑被 `/approval`、`/btw`、`/resume`、`/fork`、session inspect/recovery/export/restore、status/usage/trace 与 verify/handoff 共享，不应继续留在 `/session` 主分发文件。
   - 结果：新增 `src/commands/session_selection.rs`，迁移 session selection owner、`SessionFallbackKind`、会话 metadata JSON、inspection fallback、scoped list/action parser、queue action parser、approval/BTW cross-session lookup、session note prefix 与 `short_id`；`src/commands.rs` 注册并重新导出该 owner，`src/commands/session.rs` 只保留 `/session` 主分发和 restore-backup running-safe 入口委派；新增 `session_delegates_selection_owner` 契约测试防止 selection/fallback/scoped action helper 回流。
   - 目的：继续推进 harness 阶段 6 的 session 大模块瘦身，把跨命令复用的 session selection/fallback/action 契约从 session 主分发中分离；后续重点转向 support/legacy 降级策略、delivery 剩余编排复查、UI 入口剩余职责和真实终端观感验收。

226. Delivery Verify/Handoff Owner
   - 产品缺口：`src/commands/delivery.rs` 在 diff/report/review owner 拆出后仍直接拥有 `/verify` 与 `/handoff` 的 handler、verify/handoff option parser、test/env execution helper、verification session selection、test run persistence 和 fail-on-blockers 编排；这些职责属于交付验证子域，不应继续留在 `/diff`/`/review` 编排文件里。
   - 结果：新增 `src/commands/delivery_verify.rs`，迁移 `handle_verify`、`handle_handoff`、`VerifyOptions`、`HandoffOptions`、`HandoffFormat`、`parse_verify_args`、`parse_handoff_args`、verification session selection、test/env execution helper、verification test run projection 和 test run persistence；`src/commands.rs` 注册并重新导出该 owner，`src/commands/delivery.rs` 只保留 `/diff` 与 `/review` 命令编排；新增 `delivery_delegates_verify_handoff_owner` 契约测试防止 verify/handoff 编排回流。
   - 目的：继续推进 harness 阶段 6 的 delivery 大模块瘦身，让 delivery 集群的 diff projection、report projection、review heuristic 与 verify/handoff execution 都有独立 owner；后续重点转向 support/legacy 降级策略、UI 入口剩余职责和真实终端观感验收。

227. UI Task Monitor Shell Owner
   - 产品缺口：`src/ui.rs` 在 chat view、monitor projection、Changes/Tools/Health/Library/Result/Trace、running commands、resume picker、approval、palette 和 credential prompt 拆出后，仍直接拥有 task monitor 的文本拼装、tab strip、quick-action 聚合、点击命中和面板截断逻辑；这些职责不是 TUI 主循环本身，不应继续留在入口文件里。
   - 结果：新增 `src/ui/monitor_shell.rs`，迁移 `render_task_monitor`、`format_task_monitor_text`、`format_task_overview_lines`、`monitor_quick_actions_for_tab`、`monitor_tab_strip`、`format_monitor_tabs`、`select_monitor_tab_at_position`、`clicked_monitor_quick_action_index`、`visible_panel_line_indices`、`truncate_panel_lines`、`truncate_panel_lines_with_focus` 和 `selected_monitor_quick_action_line`；`src/ui.rs` 注册该 owner，只保留外层键鼠分发、状态字段、worker/runtime 编排与通用 UI helper；新增 `ui_module_docs_cover_monitor_shell_owner` 契约测试防止 task monitor shell 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 task monitor shell 从 TUI 主入口中分离；后续重点转向 support/legacy 降级策略、UI 入口最终职责复查和真实终端观感验收。

228. Command Policy Projection Owner
   - 产品缺口：阶段 6 已有 command registry 分组与 legacy successor/policy，但 completion catalog 只输出每条命令的 group/runningSafe；外部 UI 和脚本无法直接消费“core/support/legacy/experimental 的可见性策略”和 legacy successor/policy，只能再解析文档或硬编码降级规则。
   - 结果：新增 `src/commands/command_policy.rs`，迁移 command group/legacy policy projection owner，提供 `CommandGroupPolicy`、`command_group_policy_json`、`legacy_command_policy_json` 和 `command_policy_group_policies`；`deepcli completion json` 新增 `groups[]` 与 `legacyCommands[]`，由 registry/policy metadata 派生；新增 `command_policy_owner_projects_group_and_legacy_strategy` 契约测试与 completion JSON 单测，防止策略投影漂移。
   - 目的：继续推进 harness 阶段 6 的 support/legacy 降级策略落地，把兼容策略从文档约定升级为可被 UI、脚本和 docsync 检查消费的结构化投影；后续仍可逐项评估低价值命令是否删除或进一步降级。

229. Completion Alias Legacy Policy
   - 产品缺口：completion-only alias `repl` 已在 CLI 中描述为 legacy line-based REPL，但 registry 和 completion catalog 仍把它归为 support，`legacyCommands[]` 也没有暴露它应迁移到 `tui` 的 successor/policy，导致外部 UI 仍可能把旧入口当作普通 support 入口展示。
   - 结果：`CompletionAliasMetadata` 增加 successor/policy 字段和 `legacy_completion_alias` 构造器，`repl` 改为 completion-only legacy alias 并指向 `tui`；`src/commands/command_policy.rs::legacy_command_policy_json` 合并 slash legacy 命令与 completion-only legacy alias，并通过 `surface` 区分来源；新增/更新契约测试与 completion JSON 单测防止 `repl` 回流到 support。
   - 目的：继续推进 harness 阶段 6 的历史兼容入口降级策略，把“legacy”从文案描述落实到 registry metadata 与机器可读 completion policy，减少 UI/脚本硬编码和命令面漂移。

230. UI Dashboard Snapshot Owner
   - 产品缺口：`src/ui.rs` 在 chat view、monitor shell、command palette、credential prompt、running commands、resume picker 和 approval interaction 拆出后，仍直接拥有非交互 dashboard 的 `TuiSnapshot` 数据结构与 `render_dashboard` 布局渲染；这类快照展示不是 TUI 主事件循环或 runtime 编排职责。
   - 结果：新增 `src/ui/dashboard.rs`，迁移 `TuiSnapshot` 和 `render_dashboard`；`src/ui.rs` 注册 dashboard owner 并 re-export 公开 API，删除 dashboard 专用 ratatui import；新增 `ui_module_docs_cover_dashboard_owner` 契约测试，防止非交互 dashboard 渲染回流到 UI 入口。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把独立快照渲染从主 TUI 事件循环中分离；后续仍需复查入口剩余 helper 是否确属外层编排，并补真实终端观感验收。

231. TUI Real Terminal Smoke Gate
   - 产品缺口：阶段 7 文档长期提示“真实终端观感需本地运行确认”，但只有 ratatui 单测和字符串级 projection 检查，没有一个可重复的真实 pty smoke gate，容易在收束 UI owner 时漏掉 alternate screen、终端尺寸、主区域可见性等真实终端问题。
   - 结果：新增 `scripts/tui-smoke`，用 `/usr/bin/script` 分配真实 pty，在临时 workspace 中启动 `target/debug/deepcli --tui`，设置 `stty rows 32 cols 100`，延迟发送 Esc 退出，并检查 capture 中的 Status、Messages、Task Monitor、Overview 和 provider 信号；新增 `ui_terminal_smoke_gate_is_documented` 契约测试，要求脚本和文档入口同步存在。
   - 目的：继续推进 harness 阶段 7 的真实终端观感验收，把“人工提醒”升级为可执行的本地 smoke gate；后续仍需复查 UI 入口剩余 helper 是否确属外层编排，并可在最终收尾时结合人工 `./scripts/deepcli tui` 复查。

232. UI Chat History Owner
   - 产品缺口：`src/ui.rs` 在 chat view 和 dashboard 拆出后仍直接拥有 `ChatLine`、session message 到 UI chat line 的角色映射、空消息过滤、长历史截断和 runtime 历史加载；这些属于聊天历史投影，不是 TUI 主事件循环或输入处理。
   - 结果：新增 `src/ui/chat_history.rs`，迁移 `ChatLine`、`TUI_HISTORY_MESSAGE_CHARS`、`chat_lines_from_runtime`、`session_messages_to_chat_lines` 和 `truncate_history_message`；`src/ui.rs` 注册该 owner 并只导入当前聊天行状态需要的类型/函数；新增 `ui_module_docs_cover_chat_history_owner` 契约测试防止 chat history projection 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 session/runtime 消息投影与主事件循环分离；后续已继续拆出 header/session monitor projection，剩余重点是 worker drain、runtime rebuild 和通用输入 helper 是否还需要更细 owner。

233. UI Session Projection Owner
   - 产品缺口：`src/ui.rs` 在 chat history 拆出后仍直接拥有 active session 引用、header 状态、运行中 SessionMonitor fallback、plan 摘要和 workspace fallback；这些是会话状态投影，不是 TUI 主事件循环职责。
   - 结果：新增 `src/ui/session_projection.rs`，迁移 `ActiveSessionRef`、`HeaderStatus`、`active_session_ref`、`sync_active_session_ref`、`session_monitor_for_state`、`header_status_for_state`、`load_active_session_header`、`load_active_session_monitor`、`session_monitor_from_session`、`summarize_plan_for_tui` 和 `workspace_for_state`；`src/ui.rs` 只保存 active session 引用并委派 owner；新增 `ui_module_docs_cover_session_projection_owner` 契约测试防止 session projection 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 active session/header/task monitor projection 与主事件循环分离；后续已继续拆出通用输入 helper，剩余重点是 worker drain 和 runtime rebuild 是否确属外层 TUI 编排。

234. UI Message Box Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 `MessageBox` / `MessageBoxAction`、buffer/cursor/history 编辑规则、粘贴插入和 prompt 输入 helper；这些输入状态机被主输入、credential prompt、approval prompt 与 command palette 复用，不属于 TUI 入口主循环。
   - 结果：新增 `src/ui/message_box.rs`，迁移 `MessageBoxAction`、`MessageBox`、`handle_prompt_input_key`、`handle_key` 和 `insert_str`；`src/ui.rs` 注册 message box owner 并只消费输入状态；新增 `ui_module_docs_cover_message_box_owner` 契约测试防止输入状态机回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把通用输入编辑模型与主事件循环分离；后续已继续拆出 worker drain，剩余重点是 runtime rebuild 是否确属外层 worker/runtime 编排。

235. UI Worker Drain Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 `WorkerDone` envelope、progress channel drain、工具日志写入、done channel runtime 写回和运行结果 chat/event 写回；这些 worker channel 消费规则与主键鼠循环、命令分发和 runtime rebuild 混在一起。
   - 结果：新增 `src/ui/worker.rs`，迁移 `WorkerDone`、`drain_progress` 和 `drain_done`；`src/ui.rs` 注册 worker drain owner，并只保留 worker spawn、input submit、`/stop` 后 abort、session pause 和 runtime rebuild；新增 `ui_module_docs_cover_worker_drain_owner` 契约测试防止 worker drain 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 worker progress/done channel 消费与主事件循环分离；后续已继续拆出 runtime rebuild，剩余重点是复查 UI 入口是否只剩外层事件循环和薄编排。

236. UI Runtime Lifecycle Owner
   - 产品缺口：`src/ui.rs` 仍直接拥有 `/stop` / running `/quit` 的 worker abort、session paused 状态写回、`task_stopped` audit event 和交互 runtime rebuild；这些属于运行中任务生命周期规则，不是主键鼠循环职责。
   - 结果：新增 `src/ui/runtime_lifecycle.rs`，迁移 `stop_running_task`、`mark_active_session_paused` 和 `rebuild_runtime_for_active_session`；`src/ui.rs` 和 running command owner 只调用该 owner；新增 `ui_module_docs_cover_runtime_lifecycle_owner` 契约测试防止 runtime lifecycle 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 stop/pause/rebuild 生命周期规则与主事件循环分离；后续重点转向 UI 入口最终职责复查和真实终端观感收尾。

237. UI External Test Module
   - 产品缺口：`src/ui.rs` 的生产入口已拆出多个 owner，但文件仍有 5600+ 行，主要因为大型 `#[cfg(test)] mod tests { ... }` 内嵌在入口文件中；这让 UI 入口职责复查和后续迁移都被测试体量遮挡。
   - 结果：新增 `src/ui/tests.rs`，将大型 UI 单测模块机械迁出；`src/ui.rs` 只保留 `#[cfg(test)] mod tests;`；新增 `ui_entrypoint_uses_external_test_module` 契约测试，防止大型 UI 单测回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，让 `src/ui.rs` 的生产职责更容易审计；后续重点转向 UI 入口最终职责复查、真实终端观感收尾和阶段 6 低价值命令降级/删除复查。

238. UI Input Submission Owner
   - 产品缺口：`src/ui.rs` 在 worker drain、runtime lifecycle、message box 和 resume picker 拆出后，仍直接拥有主输入提交、空闲期本地 TUI 命令（`/resume`、`/rename`、`/credentials set` 委派）、runtime spawn、deferred input 和 resume 结果应用；这些提交规则与键鼠事件循环混在一起，不利于入口最终职责复查。
   - 结果：新增 `src/ui/input_submission.rs`，迁移 `submit_tui_input`、`handle_tui_local_command` 和 `apply_resume_result`；`src/ui.rs` 只在主输入提交和 quick action 提交时调用该 owner，resume picker owner 复用 `apply_resume_result` 应用恢复结果；新增 `ui_module_docs_cover_input_submission_owner` 契约测试防止 input submission 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把输入提交、空闲本地命令和 resume 状态应用从主事件循环中分离；后续重点转向剩余键鼠分发、滚动状态、quick action 激活和真实终端观感收尾。

239. UI Shared Text Helper Owner
   - 产品缺口：`src/ui.rs` 在多数 projection owner 拆出后仍直接维护通用文本截断、短 ID、usage/environment 格式化、action 输出摘要和最新 action result 行提取；这些 helper 被 monitor、running commands、worker、resume picker、approval、Changes/Tools/Health/Library/Result/Trace 等多个 owner 复用，不属于 TUI 入口主循环。
   - 结果：新增 `src/ui/text.rs`，迁移 `format_optional_u64`、`format_optional_bytes`、`format_cache_hit_rate`、`format_latest_environment`、`compact_ui_text`、`format_action_event`、`latest_action_result_line`、`latest_action_result`、`non_empty_output_lines`、`first_non_empty_line` 和 `short_id`；`src/ui.rs` 只注册并导入该 owner 供现有子模块复用；新增 `ui_module_docs_cover_text_helper_owner` 契约测试防止 shared text helper 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把跨 projection 复用的文本 helper 从主入口移出；后续已继续拆出滚动状态，下一步重点转向 quick action 激活、键鼠分发是否需要更细 owner，以及阶段 6 低价值命令降级/删除复查。

240. UI Transcript/Result Scrolling Owner
   - 产品缺口：`src/ui.rs` 在 text helper 拆出后仍直接拥有 transcript/result 滚动常量、PageUp/PageDown/Ctrl-Home/Ctrl-End 键盘滚动、Result 鼠标滚动、滚动事件文案和 Result 输出行计数；这些是可独立测试的 UI 状态机，不属于 TUI 入口主事件循环。
   - 结果：新增 `src/ui/scrolling.rs`，迁移 `TRANSCRIPT_SCROLL_STEP`、`TRANSCRIPT_MOUSE_SCROLL_STEP`、`RESULT_SCROLL_STEP`、`RESULT_MOUSE_SCROLL_STEP`、`handle_transcript_scroll_key`、`handle_result_scroll_key`、`scroll_result_from_mouse`、`scroll_result`、`scroll_result_down`、`result_scroll_event`、`result_output_line_count`、`scroll_transcript` 和 `transcript_scroll_event`；`src/ui.rs` 只在键鼠事件分发时调用该 owner；新增 `ui_module_docs_cover_scrolling_owner` 契约测试防止 scrolling state machine 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 transcript/result 滚动状态机从主入口移出；后续已继续拆出 quick action 激活，下一步重点转向剩余键鼠分发、粘贴路由是否需要更细 owner，以及阶段 6 低价值命令降级/删除复查。

241. UI Quick Action Activation Owner
   - 产品缺口：`src/ui.rs` 在 scrolling owner 拆出后仍直接拥有 monitor quick action 的键盘选择、选中事件文案、edit-before-run 预填、直接提交和点击激活规则；quick action projection 已由 monitor/monitor_shell owner 负责，激活状态机继续留在入口文件会让键鼠分发与业务动作提交纠缠。
   - 结果：新增 `src/ui/quick_actions.rs`，迁移 `handle_monitor_quick_action_key`、`selected_quick_action_event`、`activate_selected_monitor_quick_action` 和 `activate_monitor_quick_action_at_row`；该 owner 复用 monitor_shell 的 action projection/click hit、input_submission 的提交入口和 command_palette 的 suggestion guard；新增 `ui_module_docs_cover_quick_action_owner` 契约测试防止 quick action activation 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 monitor quick action 选择与激活从主键鼠循环中分离；后续已继续拆出粘贴路由，下一步重点转向剩余键鼠分发、几何 hit-test helper 是否需要更细 owner，以及阶段 6 低价值命令降级/删除复查。

242. UI Paste Routing Owner
   - 产品缺口：`src/ui.rs` 在 quick action activation 拆出后仍直接拥有 paste event 路由、换行归一化、credential prompt/BTW answer/resume filter/主输入框的粘贴目标选择和 paste event 文案；同时 Tools detail 预填也复用 `normalize_pasted_text`，该 helper 不应继续寄居在入口文件。
   - 结果：新增 `src/ui/paste.rs`，迁移 `handle_tui_paste` 和 `normalize_pasted_text`；`src/ui.rs` 只在 crossterm paste event 中调用该 owner，Tools detail 预填继续通过同一归一化 helper 复用；新增 `ui_module_docs_cover_paste_owner` 契约测试防止 paste routing 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把 paste routing 和换行归一化从主事件循环中分离；后续已继续拆出 geometry helper，下一步重点转向 `src/ui.rs` 最终职责复查、真实终端观感收尾，以及阶段 6 低价值命令降级/删除复查。

243. UI Geometry Helper Owner
   - 产品缺口：`src/ui.rs` 在 paste routing 拆出后仍直接维护 `rect_contains` 和 `rect_content_row_contains`；这两个 hit-test helper 被 command palette、resume picker、approvals、Changes、monitor shell、quick actions 和入口鼠标分发共同复用，不属于 TUI 主事件循环。
   - 结果：新增 `src/ui/geometry.rs`，迁移 `rect_contains` 和 `rect_content_row_contains`；`src/ui.rs` 和各交互 owner 通过同一 owner 复用矩形/内容行命中判断；新增 `ui_module_docs_cover_geometry_owner` 契约测试防止 geometry helper 回流。
   - 目的：继续推进 harness 阶段 7 的 UI 入口瘦身，把共享 hit-test helper 从主入口移出；后续重点转向 `src/ui.rs` 最终职责复查、真实终端观感收尾，以及阶段 6 低价值命令降级/删除复查。

244. UI Entrypoint Final Boundary Audit
   - 产品缺口：多轮 UI owner 拆分后，`src/ui.rs` 已接近只剩外层编排，但缺少一个可执行契约说明“入口最终边界”具体允许保留哪些函数；如果后续再把业务 projection、输入状态机或 helper 塞回入口文件，文档和测试不会立即暴露漂移。
   - 结果：新增 `ui_entrypoint_is_final_orchestration_boundary` 契约测试，约束 `src/ui.rs` 只保留 `TuiState`、`run_basic_repl`、`run_tui`、`run_tui_loop`、`handle_tui_mouse`、`handle_tools_scroll_mouse`、`handle_tui_key` 和 `cycle_monitor_tab`；同步更新 `docs/MODULES/ui.md`、`docs/HARNESS.md`、`docs/ARCHITECTURE.md` 与 `docs/ai/HANDOFF.md`，把 UI entrypoint final orchestration boundary 写入 owner 文档和验证入口。
   - 目的：完成 harness 阶段 7 的 UI 入口最终职责审计，把入口瘦身从人工判断变成契约约束；完整 harness 目标仍未结束，后续重点转向最终真实终端观感收尾和阶段 6 低价值/重复命令删除或降级复查。

245. Command Surface Pruning Audit
   - 产品缺口：阶段 6 已有 command registry 分组、legacy successor/policy 和 parser alias metadata，但缺少一份最终删除/降级审计来说明哪些重复入口保留、哪些降级展示、当前是否存在可直接删除的公开入口；这会让“删除策略已执行”仍停留在分组实现而不是可复查结论。
   - 结果：在 `docs/COMMANDS.md` 新增“删除/降级审计”，覆盖 parser thin alias、legacy slash successor 和 completion-only alias；新增 `command_surface_pruning_audit_covers_aliases_and_legacy_entries` 契约测试，要求审计记录覆盖 `CommandRouter::command_alias_metadata()`、`legacy_command_metadata()` 和 `completion_alias_metadata()`，并记录“当前未发现可直接删除的公开入口”。
   - 目的：完成 harness 阶段 6 的低价值/重复命令删除或降级复查，把保留/降级决策从口头判断变成文档与 registry 同步约束；完整 harness 收尾剩余重点转向最终真实终端观感和全量验证。

246. Harness Refactor Final Verification
   - 产品缺口：Stage 6 删除/降级审计和 Stage 7 UI 入口边界完成后，还需要用当前仓库状态重新跑文档契约、UI 投影、真实 pty smoke、全量测试和产品 gate，才能回答“按重构文档是否完整达成”。
   - 结果：已验证 `cargo test --test mvp_contract`（73 passed）、`cargo test ui::tests --lib`（74 passed）、`cargo fmt --check`、`git diff --check`、`bash -n scripts/tui-smoke && scripts/tui-smoke`（`tui-smoke: ok`）、`cargo test --quiet`（486 lib + 73 contract + 13 integration passed）、`./scripts/deepcli preflight --quick --json`（status ok，format/diff-whitespace/selftest/doctor/privacy passed）和 `./scripts/deepcli round --json`（ready true，gaps empty）。
   - 目的：本轮 `docs/ai/HARNESS_REFACTOR_PLAN.md` 对应的 harness 化重构切片可以判定完成；`docs/ai/CONTEXT.md` 里的长期 SOTA 产品循环目标仍未完成，后续应作为新的产品迭代继续推进。

247. Local `/cmd` Shell Fallback
   - 产品缺口：原生终端聊天里用户想直接执行少量 bash 命令时，只能让模型代为调用 shell 工具，或者切到外部终端；这会消耗 provider turn，也让本地命令输出无法作为当前 UI 的一等结果。
   - 结果：新增 `/cmd <bash command>` 和顶层 `deepcli cmd <bash command>`，复用本地 `run_shell` 工具、权限策略、超时和工具审计，在当前 workspace 执行命令并回显 command、exit code、stdout、stderr；新增 `/cmd --attach <bash command>`，先本地执行，再把格式化输出作为下一条 user message 进入模型上下文。
   - 行为：普通 `/cmd` 不调用 provider、不写用户消息；`--attach` 明确调用 provider，返回文本先展示本地命令输出再展示模型响应；`/cmd` 保留 raw bash tail，避免 `$HOME`、管道、重定向等 shell 语法被 slash parser 重组转义。
   - 目的：给用户一个明确、受控、可审计的 shell fallback 入口，减少“让模型执行本地小命令”的额外往返，同时保留需要模型分析命令结果时的显式 attach 模式。

## 下一步建议

- 本轮 harness 化重构切片已完成并通过 gate；后续不要继续把它当作未完成的 Stage 6/7 任务重复执行。
- 长期 SOTA 产品循环目标仍未完成；下一轮应重新做产品缺口评估，优先考虑此前明确延后的上下文压缩重构或 LLM wiki，并先单独出计划和 challenge。
- 如果需要提交本轮改动，提交前再次检查 `git status --short`、外发 diff、敏感信息和本地产物，保持 `.deepcli/benchmarks/`、exports、support bundle、credentials、logs、sessions 不进提交。
