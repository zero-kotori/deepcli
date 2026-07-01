# deepcli HARNESS 重构交接（历史）

更新时间：2026-06-28（完成全部命令 handler 拆分：goal / diagnose / doctor / recipes / opportunities / 产品循环核心 / session / env / delivery）

本文件记录 2026-06-28 阶段 3 交接状态；最新重构状态以 `docs/ai/CONTEXT.md`、`docs/HARNESS.md` 和 `docs/ARCHITECTURE.md` 为准。

最新补充：后续已陆续将 command group/legacy policy projection 拆到 `src/commands/command_policy.rs`，并将 completion-only legacy alias（如 `repl`）纳入 `legacyCommands[]` successor/policy 投影；将 delivery diff/report/review/verify-handoff owner 拆到 `src/commands/delivery_*.rs`，将 session catalog/restore/inspect/recovery/export/rename/resumable/selection owner 拆到 `src/commands/session_*.rs`，并将 UI 大型单测外置到 `src/ui/tests.rs`，将 chat view、chat history、session projection、message box、shared text helper、scrolling、quick action activation、paste routing、geometry helper、worker drain、runtime lifecycle、dashboard snapshot、command palette、credential prompt、input submission、monitor tab catalog/metadata、静态与部分动态 quick actions、SessionMonitor-only formatter、Health/Library/Result/Trace projection、Changes workspace/session diff projection、Tools tool-log projection、task monitor shell 拆到 `src/ui/*.rs` owner；`src/ui.rs` 已复查并锁定为 UI entrypoint final orchestration boundary，只保留 `TuiState`、`run_basic_repl`、`run_tui`、`run_tui_loop`、`handle_tui_mouse`、`handle_tools_scroll_mouse`、`handle_tui_key` 和 `cycle_monitor_tab`；命令面删除/降级审计已写入 `docs/COMMANDS.md` 并由 `command_surface_pruning_audit_covers_aliases_and_legacy_entries` 契约测试覆盖；真实终端观感 smoke gate 已新增并验证为 `scripts/tui-smoke: ok`；本轮 harness 化重构切片已通过全量测试、preflight quick 和 round gate，长期 SOTA 产品循环目标仍需后续迭代。

## 当时停止点

`src/commands.rs` 的**全部命令 handler 已拆分完毕**。本轮按 `docs/ai/HARNESS_REFACTOR_PLAN.md` 的阶段 3（按领域拆分源码）逐簇提取，每次都遵循红-绿流程并验证零回归。工作树在最近一次提交后保持干净。长期目标仍是该计划描述的 HARNESS 重构，**不要把本交接当作整个目标的完成**。

`src/commands.rs` 从约 36.1k 行降到约 17.3k 行（本轮移除约 18.8k 行 / 52%）；其中非测试代码已基本只剩命令分发与共享 helper，剩余体量主要是文件内的大型测试模块。

## 最近提交

- `37f9017 refactor: split command delivery handler`
- `8ec913a refactor: split command env handler`
- `d2c8105 refactor: split command session handler`
- `a6a1b8e refactor: split product-loop core into productloop module`
- `9be6096 refactor: split command opportunities handler`
- `07a14c8 refactor: split command recipes handler`
- `1fce47d refactor: split command doctor handler`
- `8de0aee refactor: split command diagnose handler`
- `29ddd35 refactor: split command goal handler`

## 已完成内容

本轮共新增 9 个命令模块（均更新了 `docs/MODULES/commands.md` 所有权说明，并在 `tests/mvp_contract.rs::commands_module_docs_cover_split_source_files` 中加入存在性契约）：

- `src/commands/goal.rs`：`/goal` 的 show/start/clear/status/gate、目标契约与守护计划生成、readiness、目标会话选择；`/round` 经 crate 内 re-export 复用其 readiness 逻辑。
- `src/commands/diagnose.rs`：`/diagnose` 与 `/support`，委派给 `/doctor` 和 `/session` handler。
- `src/commands/doctor.rs`：`/doctor` 与 `/init`，含 provider readiness/在线探测、修复、健康检查报告。
- `src/commands/recipes.rs`：`/recipes`，产品循环叶子，经 `super::` 复用 `build_round_report`、`scorecard_*`、`ScorecardOpportunity` 等。
- `src/commands/opportunities.rs`：`/opportunities`，同为产品循环叶子。
- `src/commands/productloop.rs`：产品循环核心三元组 `/scorecard`+`/round`+`/benchmark`（约 7.7k 行整体迁移，互依保持在模块内部）；`commands.rs` re-export 其他模块消费的约 18 个符号 + 一个 `#[cfg(test)]` 块。
- `src/commands/session.rs`：`/session` 主分发和 running-safe handler；后续已将 catalog/list/search/prune-empty 拆到 `src/commands/session_catalog.rs`，restore-backup 拆到 `src/commands/session_restore.rs`，show/history/summary/tools/tests/diffs/backups inspect projection 拆到 `src/commands/session_inspect.rs`，next/diagnose recovery projection 拆到 `src/commands/session_recovery.rs`，export parser/path safety/JSON 写出拆到 `src/commands/session_export.rs`，rename parser/title update 拆到 `src/commands/session_rename.rs`，可恢复会话筛选拆到 `src/commands/session_resumable.rs`，selection/fallback/scoped action helper 拆到 `src/commands/session_selection.rs`。
- `src/commands/env.rs`：`/env` check/plan/setup/install/test；当时 shared 的 `environment_next_actions`/`dedup_preserve_order` 仍留在 `commands.rs`，后续已分别迁移到 `src/commands/environment_actions.rs` 与 `src/commands/shared.rs`。
- `src/commands/delivery.rs`：变更交付簇的 `/diff` 与 `/review` 命令编排；后续已将 diff projection 拆到 `src/commands/delivery_diff.rs`，将 verify/handoff report builder 与 Markdown/PR/JSON 投影拆到 `src/commands/delivery_reports.rs`，将 review risk detection 与 sensitive/dangerous/panic-prone finding projection 拆到 `src/commands/delivery_review.rs`，将 `/verify`/`/handoff` 选项解析、test/env execution helper 与 verification session selection 拆到 `src/commands/delivery_verify.rs`。

迁移机制（已验证稳定）：`use super::*;` 头 + 显式外部 crate 导入；用 `sed` 物理迁移代码块（不手抄）；只把分发/测试/其它模块引用的符号标 `pub(crate)`；仅测试用的 re-export 用 `#[cfg(test)]` 门控；删除源区间时按降序、保留交错的共享 helper。

## 验证方法

每次拆分的红-绿流程：先在契约测试加入新模块路径观察 RED（“… should exist for command module ownership”），再迁移代码、加 `mod` 与 re-export、同步 `docs/MODULES/commands.md`，转 GREEN。

Windows 回归证明：`cargo test commands::tests --lib` 有 **23 个预先存在的平台相关失败**（执行真实 POSIX shell / git / cargo 的测试，如 `verify_*`、`benchmark_*`、`git_status_*`、`doctor_shell_*`、`test_run_*`、`global_diagnose_bundle_*` 等），它们在 `git stash -u` 的原始基线上同样失败。每次拆分都用“提取前后失败测试名集合完全一致”来证明零回归（无新增失败、无意外修复）。提交前必跑：`cargo fmt --check`、`mvp_contract` 套件、敏感内容扫描、失败集 diff。

## 剩余工作

阶段 3 的命令 handler 拆分已全部完成。后续仍属 `HARNESS_REFACTOR_PLAN.md` 的范围：

- 收尾清理：`productloop.rs`/`session.rs`/`env.rs`/`delivery.rs` 用了 `use super::*;` glob 与少量“寄居”共享 helper（如 `local_action_checklist` 落在 `productloop`），属于所有权小瑕疵，可在后续收紧为显式导入并归位。
- 阶段 2 去硬编码：把散落的命令清单/别名/help/running-safe/schema version/阈值等迁移到有所有权的 registry 或 typed config。
- 阶段 4-5 文档瘦身与 docsync：把长文档收束为总览 + 模块说明 + ADR；扩展 docsync 检查（命令清单、模块文档、过时文档）。
- 后续产品循环：本轮 harness 化重构切片已完成；长期 SOTA 目标仍需下一轮重新做产品缺口评估，优先考虑此前明确延后的上下文压缩重构或 LLM wiki。
- 大型文件内测试模块：当时 `commands.rs` 仍内嵌大量测试，后续已迁移到 `src/commands/tests.rs`。

## 提交前检查清单

- `git status --short`
- `cargo fmt --check`
- `cargo test commands::tests --lib`（与 23 个 Windows 基线失败对照，不得引入新失败）
- `cargo test --test mvp_contract`
- `./scripts/deepcli preflight --quick --json`
- 暂存改动时 `git diff --cached --check`
- 扫描暂存/外发改动中的本地产物、凭证、会话、日志、benchmark 证据、support bundle 和疑似敏感 token
- 期望提交身份 `zero-kotori <kotorizero8@gmail.com>`

已知限制：`preflight --quick` 故意跳过 clippy 和 gate；那 23 个基线测试失败是平台相关（Windows 上跑 POSIX shell / git / cargo），不是重构回归。
