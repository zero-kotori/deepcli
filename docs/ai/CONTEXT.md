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
   - 结果：当 `deepcli scorecard --json` 没有 gaps 且状态为 ok 时，顶层 `nextActions` 会切换为持续验收动作：`deepcli round --json`、`deepcli preflight --json`、`deepcli gate --json`、`deepcli recipes sota --json`、`deepcli benchmark trends --json`、`deepcli benchmark status --json` 和 baseline compare。
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

## 当前产品自评

当前本地自评中，`scorecard` 为 80/80，`benchmark status` 为 ready；如果本地 `.deepcli/benchmarks/` 只有每个 required case 的单条样本，`benchmark trends` 会返回 `insufficient_history`，`round` 会据此进入 `needs_attention` 并提示 `deepcli round --json --run-benchmark --fail-on-command`。该结果依赖 `.deepcli/benchmarks/` 下的本地忽略证据 artifact，这些文件不应推送到远程仓库。

如果 fresh checkout 或清理后缺少本地 benchmark evidence，可通过 `deepcli round --json --run-benchmark --fail-on-command` 重新生成，使本地 `scorecard` 达到 80/80、`benchmark status` 为 ready；这些 `.deepcli/benchmarks/` artifact 仍然只作为本地证据，不进入 Git 提交。

下一轮产品设计应继续从真实使用阻力中选一个高价值缺口，而不是只为了让分数变绿而提交本地 artifact；本轮已补齐 baseline 模板未填写时的 compare 引导、scorecard 分类级 nextActions 排序、round 摘要中的分类级 nextActions 透传、scorecard 全局 nextActions 的 gap-aware 聚焦、scorecard nextActions 的可执行 CLI 命令格式、benchmark preset gap 修复提示的可执行 CLI 命令格式、recipes nextActions 的可执行 CLI 命令格式、scorecard nextActions 的自引用跳转清理、round nextActions 的自引用跳转清理、benchmark status 空证据状态的 clean action 隐藏、scorecard benchmark 修复队列的 round 只读跳转回归测试、fork 上下文复制透明化、benchmark trends 文本证据格式修复、scorecard ready 状态下的下一步动作聚焦、benchmark trends 单样本历史不足状态、round 聚合 benchmark trends gate、round benchmark trends 修复动作闭环、顶层命令帮助旗标转发、benchmark trends 历史不足闭环动作、SOTA recipe 状态感知 nextActions、scorecard ready 状态感知 trend 修复动作、TUI 运行中产品循环观察命令、TUI running-safe 标记收敛、TUI 运行中 fork 持久化上下文、terminal dry-run 可验收报告、fork dry-run 预览、fork resume 健康检查、resume dry-run 预览、resume 候选去噪、preflight 运行诊断摘要、benchmark status/summary JSON 内嵌 report，以及 resume 低信息澄清会话去噪，下一轮可继续关注 benchmark evidence 运行体验、TUI 可观测性或恢复历史的真实交互阻力。

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
./scripts/deepcli resume --dry-run --json
./scripts/deepcli fork --current --dry-run --json
./scripts/deepcli fork --current --no-open --verify --json
./scripts/deepcli terminal --dry-run --json
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
