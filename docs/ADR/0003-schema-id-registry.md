# ADR 0003：稳定 JSON schema 标识符的所有权 registry

## 状态
已接受并落地。

## 背景
用户可见的 `--json` 输出都带 `"schema": "deepcli.<name>.v1"` 标签，原先以裸字符串字面量散落在源码中（65 个 schema、约 267 处），无统一 owner，typo 会导致单一代码路径的 schema 漂移。

## 决策
新建 `src/schema_ids.rs` 作为全部稳定 schema 标识符的单一 owner（`pub const` + 清单 + 命名/唯一性守护单测）。生产发射点统一引用常量；测试断言与 fixture 保留字面量，作为常量值的独立交叉校验（值写错则测试失败）。常量用 `pub`（crate lib 的稳定对外契约，避免未用告警并便于枚举）。

## 影响
- 全部生产发射点（productloop、命令模块、session、credentials 等）已迁移到 registry。
- schema 版本/owner 集中可查，为后续 docsync（schema 清单校验）与版本迁移策略打基础。
