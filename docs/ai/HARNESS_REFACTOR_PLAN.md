# deepcli harness 化重构计划草案

> 状态：草案，等待人工 challenge。本文只记录重构方向和约束，不代表已经开始代码实现。
> 范围：先服务 harness 化代码重构；已确认的核心功能范围记录在 `docs/ai/CONTEXT.md`，本文不新增新的产品功能范围。

## 1. 目标

下一阶段目标不是继续增加细碎命令，而是把 deepcli 从“功能堆叠较全的本地 CLI”收束成“核心链路小而精、行为可验证、文档可同步、架构边界清晰”的 AI coding CLI。

harness 化重构的核心目的：

- 把 harness 定义为轻量架构上下文：向开发 agent 提供当前模块地图、边界原则、文档同步要求和最小验证要求。
- 用模块边界和文档边界约束后续实现，避免继续把命令、schema、UI、runtime、产品报告和文档堆进少数大文件。
- 让 agent 基于这些信息自行阅读代码、追踪调用链、判断根因和选择修改位置，而不是被预设路径绑住。
- 让文档成为实现契约的一部分，代码改动必须同步更新对应说明，避免过时文档继续累积。

本文里的 harness 不是单指测试工具，也不是必须引入 fake provider 或 fake tool executor。它当前也不是严格的任务执行器或修改路由器；本轮优先提供轻量架构上下文、边界原则、文档同步约定和验证要求。

## 2. 非目标

本轮重构不做以下事情：

- 不优先新增小型 slash 命令、诊断变体、benchmark 子命令或非核心 UI tab。
- 不把 `/harness` 当作又一个普通小命令先塞进现有命令大文件。
- 不一次性大爆炸重写所有模块；重构必须能分阶段验证和回滚。
- 不引入新的大型框架来掩盖当前边界问题。
- 不在重构计划里扩展核心产品功能范围；核心功能列表以 `docs/ai/CONTEXT.md` 的当前记录为准。
- 不在本轮修改上下文压缩或 LLM wiki 的功能行为；这两项等架构重构完成后，再单独出计划并经过 challenge。

## 3. 当前问题判断

基于当前代码和文档盘点，主要问题不是“没有功能”，而是功能、命令、文档和状态契约过于集中。

当前高风险区域：

- `src/commands.rs` 同时承担命令枚举、解析、help、业务处理、JSON schema、scorecard、benchmark、session、git、privacy 等大量职责，已经成为主要复杂度入口。
- `src/ui.rs` 同时承担 TUI 状态、交互、渲染、鼠标键盘、monitor tab、running-safe 分发等职责，后续继续扩展容易破坏体验。
- `src/runtime.rs` 内含 Agent loop、provider turn、tool-call budget、现有上下文处理和 session 观测逻辑，领域边界不清；本轮只标记边界并保持现有上下文行为。
- `src/tools.rs` 以字符串注册和分发工具，工具声明、参数、权限 surface 和审计生命周期没有被统一契约化。
- `docs/ai/TECHNICAL_PLAN.md` 和 `docs/ai/REQUIREMENTS.md` 已经写入大量细节命令和 UI 行为，容易和真实实现漂移，也不适合作为模块级维护文档。

因此，harness 化重构应先解决“agent 需要哪些项目结构信息和边界原则，才能更好地自行判断问题和修改位置”，再解决具体文件迁移和功能收束。

## 4. 重构原则

- harness first：任何核心链路拆分前，先有轻量模块地图、依赖边界、文档同步和测试分层说明。
- 小步迁移：先抽契约和适配层，再迁移具体实现，避免一次性重写。
- 行为不扩张：重构阶段默认不新增产品能力，只做边界、可测性、文档同步和冗余削减。
- 核心优先：围绕会话、工具调用、沙箱、UI 投影、goal/fork/plan/harness 等已确认核心入口建立边界，不为低频小命令设计复杂架构。
- 文档同步：模块行为改变时，同步更新模块说明；删除或降级命令时，同步更新命令文档。
- 可删除优先于兼容堆叠：确认无价值或重复的旧路径应进入 legacy/support 或删除计划，而不是继续加兼容分支。
- 直接测试优先：能用单元测试、集成测试或 contract tests 直接覆盖的行为，不先引入 fake provider/fake tool executor。
- agent 自主判断：harness 不预设固定修改路径；agent 应根据真实代码调用链决定修改位置。

## 5. 阶段计划

### 阶段 0：冻结范围、命令分组和建立基线

目标：停止继续扩大命令面，明确哪些能力进入核心、support 或 legacy。

动作：

- 冻结新的细碎 slash 命令和顶层 alias，除非它直接服务 harness 验收或已确认核心功能。
- 给现有命令分组：core、support、legacy、experimental。
- 命令保留程度先由维护者决定：默认保留现有公开入口兼容，但主文档、主 UI 和后续实现只突出核心命令；低频诊断、benchmark 细项和历史兼容入口进入 support/legacy。
- 记录当前必须保持兼容的 JSON schema 和用户入口。
- 确认本轮重构的验收命令，例如 `cargo test`、现有 contract tests、文档检查和后续架构约束检查。

产出：

- 命令分组文档。
- 兼容性边界说明。
- 重构前基线报告。

### 阶段 1：建立轻量架构 harness

目标：把 deepcli 的模块地图、边界原则、文档同步规则和验证要求整理成开发 agent 可读的上下文，而不是规定固定路径。

架构 harness 组成：

- 模块地图：说明现有主要模块职责、重要文件、当前复杂度热点和已知边界问题。
- 边界原则：说明哪些跨层依赖需要谨慎，例如工具必须经过权限、UI 不应承载业务判断、命令层不应长期堆积主体逻辑。
- 判断原则：agent 应先阅读相关代码和调用链，再决定修改位置；harness 不预设固定路径。
- 文档同步规则：改行为同步相关文档，删除或降级同步删除过时文档。
- 测试分层说明：纯逻辑、命令输出、跨模块流程、UI 状态分别优先用什么测试方式。

产出：

- 文档 `docs/HARNESS.md`，说明轻量架构 harness、模块地图、边界原则、测试分层和文档同步规则。
- 第一批轻量检查：模块文档存在性、命令文档同步、过时文档删除、核心逻辑继续堆入旧大文件的风险提示。
- 必要时补少量测试替身，但它们是测试实现细节，不是 harness 的定义。

### 阶段 2：去硬编码

目标：把散落在大文件里的字符串、命令、schema、路径、阈值和 UI label 迁移到有所有权的结构。

优先处理：

- 命令清单、别名、help topic、running-safe 标记和命令分组。
- 工具声明、参数 schema、权限 surface 和审计事件名。
- JSON schema version、nextActions/checklist label 和 report section。
- provider 默认值、模型 capability 和超时。
- benchmark preset、required preset、artifact 路径和 freshness 阈值。
- 文档路径、支持包路径、session artifact 路径。

期望形态：

- 常量不是简单换位置，而是进入对应模块的 registry 或 typed config。
- registry 需要能被命令、UI、文档生成或 harness 复用。
- 用户可见 schema 需要明确 owner，避免多个函数手写同一结构。

### 阶段 3：代码功能拆分

目标：从“大 `src` + 少数巨型文件”改成按领域分层的源码结构。

建议目标结构：

```text
src/
  app/
    bootstrap.rs
    errors.rs
    output.rs
  cli/
    args.rs
    aliases.rs
    dispatch.rs
  commands/
    registry.rs
    response.rs
    core/
    goal/
    plan/
    fork/
    session/
    harness/
    support/
  runtime/
    agent_loop.rs
    events.rs
    provider_turn.rs
    tool_loop.rs
    observation.rs
  tools/
    registry.rs
    executor.rs
    schema.rs
    file.rs
    shell.rs
    git.rs
    test.rs
    environment.rs
  permissions/
    policy.rs
    sandbox.rs
    approval.rs
    audit.rs
  session/
    store.rs
    model.rs
    snapshot.rs
    migration.rs
  providers/
    traits.rs
    deepseek.rs
    kimi.rs
  ui/
    app.rs
    input.rs
    monitor/
    render/
    actions.rs
  harness/
    boundaries.rs
    ownership.rs
    checks.rs
  docsync/
    inventory.rs
    checks.rs
```

拆分顺序：

1. 先拆 command registry、命令元数据和 response builder，减少 `commands.rs` 中的硬编码。
2. 再拆工具声明和工具执行，形成 typed tool contract。
3. 然后拆 runtime 的 provider turn、tool loop 和 observation。
4. 最后拆 UI 的状态、输入、monitor model 和 render，避免 UI 在核心契约未稳定前反复移动。

拆分约束：

- 每个迁移步骤应参考架构 harness，并有现有测试或新增的最小相关测试保护。
- 迁移期间可以保留薄 wrapper，但 wrapper 只做转发，不继续增加业务逻辑。
- 新模块必须有明确 owner 文档和边界说明。
- 删除策略、模块化和测试分层是本轮必须执行的重构内容，不作为可选建议。

### 阶段 4：文档瘦身和去冗余

目标：把“所有需求都塞进长文档”的模式改成总览文档 + 模块文档 + 决策记录，并删除已经过时或和实现冲突的区域。

建议文档结构：

```text
docs/
  ARCHITECTURE.md
  HARNESS.md
  COMMANDS.md
  CORE_FEATURES.md
  MODULES/
    runtime.md
    tools.md
    session.md
    permissions.md
    ui.md
  ADR/
    0001-harness-first.md
    0002-core-command-scope.md
docs/ai/
  CONTEXT.md
  REQUIREMENTS.md
  TECHNICAL_PLAN.md
  HARNESS_REFACTOR_PLAN.md
```

文档职责：

- `README.md`：只保留快速开始、主路径和核心链接。
- `docs/ARCHITECTURE.md`：描述当前真实架构，不放长命令列表。
- `docs/HARNESS.md`：描述轻量架构 harness、模块地图、边界原则、测试分层、文档同步规则和检查命令。
- `docs/COMMANDS.md`：记录命令分组、稳定入口、support/legacy 状态。
- `docs/CORE_FEATURES.md`：只写核心功能契约，不收录所有边缘命令。
- `docs/MODULES/*.md`：每个核心模块一份说明，包括职责、输入输出、关键类型、测试入口和文档同步要求。
- `docs/ADR/*.md`：记录重构中不可逆或有争议的架构决策。
- `docs/ai/CONTEXT.md`：只记录当前决策和 handoff，不继续沉淀完整规格。
- `docs/ai/REQUIREMENTS.md` 和 `docs/ai/TECHNICAL_PLAN.md`：重构时瘦身，删除过时区域，只保留历史背景、高层方向和仍有效的核心约束，不再作为细节命令数据库。

### 阶段 5：禁止过时文档

目标：让文档漂移成为可检查问题，而不是靠记忆维护。

规则：

- 改核心模块行为，必须同步更新对应 `docs/MODULES/*.md`。
- 改命令入口、别名、输出 schema 或命令分组，必须同步更新 `docs/COMMANDS.md`。
- 改架构 harness 的模块地图、边界原则、测试分层或检查规则，必须同步更新 `docs/HARNESS.md`。
- 改核心功能范围或阶段性产品决策，必须同步更新 `docs/ai/CONTEXT.md`。
- 删除、降级或迁移旧能力时，必须在文档里删除旧承诺或标注 legacy/support，不保留自相矛盾描述。
- 每次完成需求、修复或重构后，必须同步更新受影响文档；如果文档不需要更新，也应在验收说明里说明原因。

可执行检查：

- 增加 docsync 检查，校验命令 registry 与 `docs/COMMANDS.md` 的命令清单一致。
- 校验核心模块存在对应 `docs/MODULES/*.md`。
- 校验公开 JSON schema owner 有文档入口。
- 在 CI 或 preflight 中加入文档同步检查，至少覆盖命令清单、模块文档和 harness 文档。

### 阶段 6：删除策略、模块化和测试分层落地

目标：把已经确认要执行的工程约束落到重构流程里。

删除策略：

- 识别重复命令、低价值命令、历史兼容入口和过时文档承诺。
- 对仍需兼容的入口标记 legacy/support，并从主文档和主 UI 中移出。
- 对确认无价值或与当前方向冲突的实现和文档直接删除，不保留“已过时但继续描述”的区域。

模块化：

- 每类功能修改应由 agent 沿真实代码调用链自行判断落点，避免继续向旧大文件追加主体逻辑。
- 旧大文件迁移后只保留 registry、薄适配层或 re-export。
- 新模块必须有 owner 文档、边界说明和最小测试入口。

测试分层：

- unit tests：覆盖纯函数、解析、权限判定、路径校验、schema builder。
- contract tests：覆盖稳定 CLI 命令、JSON schema、nextActions 和 checklist。
- integration tests：覆盖跨模块流程，例如 session + command + tools + permissions。
- UI projection tests：覆盖 UI 需要消费的状态模型，不优先测试终端字符串细节。
- docsync checks：覆盖命令清单、模块文档和过时文档删除。

### 阶段 7：UI 收束

目标：让 UI 服务核心任务体验，而不是展示所有诊断面板。

建议保留主视图：

- Overview：任务状态和下一步。
- Tools：工具调用和失败工具。
- Changes：工作区变化和 diff。
- Tests：测试证据。
- Session：会话、goal、plan、fork 状态。
- Approvals：审批和旁路问题。
- Context：只保留现有上下文状态展示；不在本轮新增上下文压缩或 LLM wiki 行为。

处理方式：

- 其它诊断视图进入 advanced/support，而不是继续占用主 tab。
- UI 不直接拼复杂业务逻辑，只消费 runtime、session 和相关领域模块暴露的 projection model。
- 架构 harness 建议 UI 消费 projection model，不依赖终端渲染字符串作为核心状态来源；具体修改落点仍由 agent 结合代码判断。

## 6. 已确认执行的补充约束

以下约束纳入本轮重构，不再作为可选建议：

- 兼容策略：明确哪些命令和 JSON schema 必须稳定，哪些可以标记 legacy，避免无意识破坏用户脚本。
- schema versioning：所有稳定 JSON 输出明确 owner 和版本迁移策略。
- 删除策略：对重复、低价值、小众命令建立删除或降级流程，避免继续无限兼容。
- 测试分层：unit tests 保护纯逻辑，contract tests 保护命令输出，integration tests 保护跨模块流程，projection tests 保护 UI 状态。
- 模块边界原则：UI 不直接读写工具，runtime 不直接拼 UI 文案，tools 不直接绕过 permissions，commands 不直接管理 session 内部格式；如确需偏离，应说明原因。
- 观测一致性：session、trace、UI projection 和 JSON report 使用同一批事件和状态来源，避免四套状态各说各话。
- 迁移日志：每个阶段记录迁移了哪些入口、保留了哪些 wrapper、删除了哪些旧承诺。
- CI gate：重构完成后，preflight 至少应覆盖格式、测试、架构 harness 检查、文档同步和隐私扫描。
- 延后功能：上下文压缩重构和 LLM wiki 不进入本轮实现计划，只保留现有行为；后续单独计划、单独 challenge、单独实现。

## 7. 建议验收标准

一轮 harness 化重构完成后，应满足：

- agent 能从 harness 文档获得清晰模块地图、边界原则、测试入口和文档同步要求。
- 主要命令入口由 registry 驱动，help、alias、running-safe 和 docs 不再各写一份。
- 工具声明、权限 surface 和参数 schema 可被同一套契约描述。
- UI 主视图倾向消费 projection model，核心状态不依赖终端渲染字符串作为唯一来源。
- 文档结构从长文档堆叠收束为总览、模块说明、命令说明、harness 说明和 ADR。
- 文档同步检查能发现至少一类真实漂移，例如新增命令但未更新命令文档。
- 过时需求和过时技术方案被删除或迁出主文档，不继续保留冲突描述。

## 8. 待 challenge 的关键问题

上一轮 challenge 已确认的决策：

- harness 定义为轻量架构上下文，不定义为 fake provider/fake tool executor 集合，也不预设固定修改路径。
- 上下文压缩重构和 LLM wiki 本轮不做，等架构重构完成后单独计划。
- 删除策略、模块化和测试分层必须执行。
- 命令保留程度先由维护者决定，后续重点复查重要命令实现。
- 源码结构按领域拆分。
- 文档瘦身，删除过时区域；每次需求或修复完成后同步更新相关文档。
- UI 按核心视图收束，非核心诊断进入 advanced/support。

后续仍需 challenge 的问题：

- 第一版 command core/support/legacy 分组是否合理。
- 第一批被删除或降级的命令、文档段落和兼容 wrapper 是否有误伤。
- 架构 harness 检查应该先做到 warning，还是在第一阶段就作为强 gate。
- 文档瘦身时哪些历史背景需要保留，哪些可以直接删除。
