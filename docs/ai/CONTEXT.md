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
   - 行为：该动作排在手工 `.deepcli/baselines/competitor.json` 模板之前；如果默认 competitor baseline 已存在，仍只推荐 `deepcli benchmark compare --baseline .deepcli/baselines/competitor.json --json`；如果当前 artifact 缺失或缺少 duration，则仍只推荐手工 baseline template。
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
   - 结果：`deepcli cleanup sessions --json` 和 `deepcli session prune-empty --json` 顶层 `nextActions` 改为 `deepcli cleanup sessions --force`、`deepcli session list ...`、`deepcli history ...` 这类 shell 可执行命令。
   - 行为：dry-run 仍只预览候选并跳过当前会话和有标题空会话；说明性清理摘要留在 `report`，不把 TUI slash 命令放入 JSON 顶层动作。
   - 目的：用户从 resume/fork 无源路径进入历史清理时，可以直接复制 JSON 动作执行维护命令，不需要把 `/session prune-empty --force` 手动改写成顶层 CLI。

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

## 当前产品自评

当前本地自评中，`scorecard` 为 80/80，`benchmark status` 为 ready；如果本地 `.deepcli/benchmarks/` 只有每个 required case 的单条样本，`benchmark trends` 会返回 `insufficient_history`，`round` 会据此进入 `needs_attention` 并提示 `deepcli round --json --run-benchmark --fail-on-command`。当 benchmark evidence、trends 和 goal gates 都 ready 时，`round.nextActions` 会继续在 preflight/gate 后提示 `--from-current`、手工 baseline template 或 baseline compare；如果默认 competitor baseline 缺失且本地 artifact 可完整捕获，会先提示 `baseline-template --from-current` 生成 `status=ready` 的 compare-ready baseline，再提示手工 competitor baseline template。该结果依赖 `.deepcli/benchmarks/` 下的本地忽略证据 artifact，这些文件不应推送到远程仓库。

如果 fresh checkout 或清理后缺少本地 benchmark evidence，可通过 `deepcli round --json --run-benchmark --fail-on-command` 重新生成，使本地 `scorecard` 达到 80/80、`benchmark status` 为 ready；这些 `.deepcli/benchmarks/` artifact 仍然只作为本地证据，不进入 Git 提交。

下一轮产品设计应继续从真实使用阻力中选一个高价值缺口，而不是只为了让分数变绿而提交本地 artifact；本轮已补齐 baseline 模板未填写时的 compare 引导、baseline-template 自带后续动作、baseline-template 捕获当前 benchmark、ready 产品循环优先推荐当前 baseline 捕获、Fork/Terminal 可选终端 app、Fork/Terminal 终端 app 默认偏好、Wrapper terminal 帮助可发现性、Wrapper Git inspect 帮助可发现性、Wrapper 协作队列帮助可发现性、fork workspace-aware 恢复命令、Fork nextActions workspace-aware、Resume 空候选 JSON 错误结构化、Session search JSON nextActions、Fork current shell 误用动作直达、Diagnose/Support JSON actions 可执行化、Completion JSON actions 可执行化、Support bundle manifest actions 可执行化、Terminal opened actions 可执行化、Git read-only JSON 输出、Git inspect output artifact、TUI 运行中 read-only Git inspection、TUI 运行中旁路命令 artifact guard、TUI 运行中 completion force install guard、Verify/Handoff JSON actions 可执行化、Fork 无源错误 nextActions 候选发现、Fork no-source actions 去占位符、Inspect JSON actions 去占位符、Test JSON actions 可执行化、Next JSON 动作可执行化、Benchmark aging 顶层刷新动作、Quickstart/Selftest JSON 动作可执行化、Cleanup JSON 动作可执行化、Health/Version JSON 动作可执行化、Inspect JSON 动作可执行化、Running Fork JSON 动作可执行化、Quick Preflight 隐私快路径、Status session actions 可执行化、Usage session actions 可执行化、restore-backup 结构化预览、TUI 运行中 restore-backup 安全预览、TUI 运行中 `/session` read-only guard、环境 JSON nextActions 可执行化、Tools 视图工具输出动作可见化、Quick actions run/edit 语义提示、Terminal workspaceCommand、scorecard 分类级 nextActions 排序、round 摘要中的分类级 nextActions 透传、scorecard 全局 nextActions 的 gap-aware 聚焦、scorecard nextActions 的可执行 CLI 命令格式、benchmark preset gap 修复提示的可执行 CLI 命令格式、recipes nextActions 的可执行 CLI 命令格式、scorecard nextActions 的自引用跳转清理、round nextActions 的自引用跳转清理、benchmark status 空证据状态的 clean action 隐藏、scorecard benchmark 修复队列的 round 只读跳转回归测试、fork 上下文复制透明化、benchmark trends 文本证据格式修复、scorecard ready 状态下的下一步动作聚焦、benchmark trends 单样本历史不足状态、round 聚合 benchmark trends gate、round benchmark trends 修复动作闭环、顶层命令帮助旗标转发、benchmark trends 历史不足闭环动作、SOTA recipe 状态感知 nextActions、scorecard ready 状态感知 trend 修复动作、TUI 运行中产品循环观察命令、TUI running-safe 标记收敛、TUI 运行中 fork 持久化上下文、terminal dry-run 可验收报告、fork dry-run 预览、fork resume 健康检查、resume dry-run 预览、resume 候选去噪、preflight 运行诊断摘要、benchmark status/summary JSON 内嵌 report、resume 低信息澄清会话去噪、resume 当前 workspace 与短任务去噪、privacy 配置化禁用词扫描、fork 默认候选去噪、fork JSON 错误结构化、benchmark 证据 freshness 可见性、SOTA recipe baseline-aware nextActions、scorecard ready baseline-aware nextActions，以及 round ready baseline-aware nextActions，下一轮可继续关注 TUI 可观测性、恢复历史或环境自动化验收的真实交互阻力。

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
./scripts/deepcli fork --current --dry-run --json
./scripts/deepcli fork --current --no-open --verify --json
./scripts/deepcli terminal --dry-run --json
DEEPCLI_TERMINAL_APP=iTerm2 ./scripts/deepcli terminal --dry-run --json
./scripts/deepcli terminal --app iTerm2 --dry-run --json
./scripts/deepcli --help | rg 'terminal'
./scripts/deepcli --help | rg 'git status'
./scripts/deepcli git status --json
./scripts/deepcli --help | rg 'approval list|btw ask'
./scripts/deepcli approval list --json
./scripts/deepcli btw list --json
./scripts/deepcli sessions -h
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
./scripts/deepcli benchmark baseline-template --from-current --name current-main --output .deepcli/baselines/current-main.json --json
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
