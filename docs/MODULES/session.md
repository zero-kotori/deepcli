# 会话模块

## 职责

`src/session.rs` 负责持久化的会话元数据与记录：消息、审计事件、计划、目标、审批、附带问题、工具调用、测试、diff、备份，以及按 id 或前缀进行的会话查找。

## 边界

- 其他模块应使用 `SessionStore` 与会话模型方法，而不是临时直接写文件。
- schema 变更需要迁移或向后兼容的读取器。
- 运行时可以追加观测与状态变更，但命令与 UI 界面应消费会话 API，而不是直接解析存储文件。
- fork、resume、goal、plan、approval 以及 by-the-way 流程应保持持久化上下文语义。

## 测试

- 针对性的 `session::tests::*`，覆盖存储、前缀查找、元数据、记录以及向后兼容性。
- 针对 `/session`、`/resume`、`/fork`、`/goal`、`/plan`、`/approval` 与 `/btw` 的命令契约测试。
- 针对运行安全的会话投影的 UI 测试。

## 文档同步

当会话存储结构、选择规则、fork/resume 语义或迁移规则发生变化时，更新本文件。对于公开命令或 schema 变更，更新 `docs/COMMANDS.md`；对于重大产品决策，更新 `docs/ai/CONTEXT.md`。
