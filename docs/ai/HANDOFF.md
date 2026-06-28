# deepcli HARNESS 重构交接

更新时间：2026-06-28（完成全部命令 handler 拆分：goal / diagnose / doctor / recipes / opportunities / 产品循环核心 / session / env / delivery）

## 当前停止点

`src/commands.rs` 的**全部命令 handler 已拆分完毕**。本轮按 `docs/ai/HARNESS_REFACTOR_PLAN.md` 的阶段 3（按领域拆分源码）逐簇提取，每次都遵循红-绿流程并验证零回归。工作树在最近一次提交后保持干净。长期目标仍是该计划描述的 HARNESS 重构，**不要把本交接当作整个目标的完成**——后续阶段（去硬编码、文档瘦身、docsync、UI 收束）尚未开始。

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
- `src/commands/session.rs`：`/session` 全部子命令、running-safe handler、restore-backup、可恢复会话筛选与投影；re-export 约 31 个会话 helper 供 resume/fork/approval/btw/verify 经 `super::` 使用。
- `src/commands/env.rs`：`/env` check/plan/setup/install/test；shared 的 `environment_next_actions`/`dedup_preserve_order` 仍留在 `commands.rs`。
- `src/commands/delivery.rs`：变更交付簇 `/diff`+`/review`+`/verify`+`/handoff`（约 3.25k 行整体迁移，verify→review/diff、handoff→verify 的互依保持在模块内部）。

迁移机制（已验证稳定）：`use super::*;` 头 + 显式外部 crate 导入；用 `sed` 物理迁移代码块（不手抄）；只把分发/测试/其它模块引用的符号标 `pub(crate)`；仅测试用的 re-export 用 `#[cfg(test)]` 门控；删除源区间时按降序、保留交错的共享 helper。

## 验证方法

每次拆分的红-绿流程：先在契约测试加入新模块路径观察 RED（“… should exist for command module ownership”），再迁移代码、加 `mod` 与 re-export、同步 `docs/MODULES/commands.md`，转 GREEN。

Windows 回归证明：`cargo test commands::tests --lib` 有 **23 个预先存在的平台相关失败**（执行真实 POSIX shell / git / cargo 的测试，如 `verify_*`、`benchmark_*`、`git_status_*`、`doctor_shell_*`、`test_run_*`、`global_diagnose_bundle_*` 等），它们在 `git stash -u` 的原始基线上同样失败。每次拆分都用“提取前后失败测试名集合完全一致”来证明零回归（无新增失败、无意外修复）。提交前必跑：`cargo fmt --check`、`mvp_contract` 套件、敏感内容扫描、失败集 diff。

## 剩余工作

阶段 3 的命令 handler 拆分已全部完成。后续仍属 `HARNESS_REFACTOR_PLAN.md` 的范围：

- 收尾清理：`productloop.rs`/`session.rs`/`env.rs`/`delivery.rs` 用了 `use super::*;` glob 与少量“寄居”共享 helper（如 `local_action_checklist` 落在 `productloop`），属于所有权小瑕疵，可在后续收紧为显式导入并归位。
- 阶段 2 去硬编码：把散落的命令清单/别名/help/running-safe/schema version/阈值等迁移到有所有权的 registry 或 typed config。
- 阶段 4-5 文档瘦身与 docsync：把长文档收束为总览 + 模块说明 + ADR；扩展 docsync 检查（命令清单、模块文档、过时文档）。
- 阶段 7 UI 收束：UI 改为消费 projection model。
- 大型文件内测试模块：`commands.rs` 仍内嵌大量测试，后续可考虑随模块迁移到各自的 `#[cfg(test)]` 子模块。

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
